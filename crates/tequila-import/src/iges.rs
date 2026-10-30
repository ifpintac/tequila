//! IGES (5.3), the neutral CAD format before STEP, read in the subset CAD
//! packages still write: surfaces and solids.
//!
//! What comes in:
//!
//! * trimmed surfaces (type 144), the form SolidWorks, Rhino, Fusion and
//!   most others write by default: NURBS surfaces (128) and, less often,
//!   planes, ruled surfaces, surfaces of revolution and tabulated cylinders
//!   (108, 118, 120, 122), each bounded by curves on it (142);
//! * solids written as manifold solid B-rep objects (186), over NURBS and
//!   IGES 5.3's analytic planes, cylinders, cones, spheres and tori
//!   (190 to 198);
//! * untrimmed surfaces standing alone, and bounded planes;
//! * the curves that bound them: lines, circular arcs, ellipses,
//!   polylines, composite curves, parametric splines and NURBS (110, 100,
//!   104, 106, 102, 112, 126);
//! * transformation matrices, colours (by number or a 314 definition),
//!   names (406 form 15, or an entity's label), subfigures (308 placed by
//!   408, meshed once and shared), and the length unit.
//!
//! A surface model has no bodies, only so many separate faces. Faces that
//! meet at their corners and share a colour are gathered into one part,
//! which is what the model drew as one body; a solid is one part.
//!
//! Each face is divided in its own parameter space: constrained Delaunay
//! over its trimming loops and a grid fine enough for the chord tolerance,
//! with normals from the exact surface. The quality presets are STEP's, and
//! the file is kept with the project to be divided again. A face's
//! boundary curves are its edges, drawn when model edges are shown; a
//! closed surface's seam is not an edge and is left out.
//!
//! Not read: offset curves and surfaces, and the drafting entities.
//! Compressed and binary IGES are refused by name.

use crate::raw::{RawMesh, RawNode, RawScene};
use crate::step::Tessellation;
use crate::ImportError;
use tequila_core::math::{vec3, Mat4, Transform};
use tequila_core::spectrum::Spectrum;
use tequila_core::units::LengthUnit;
use tequila_materials::material_type::MaterialType;
use tequila_render_core::material::Material;
use tequila_scene::material::MaterialDefinition;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::f64::consts::{PI, TAU};
use std::sync::Arc;

type P3 = [f64; 3];
type P2 = [f64; 2];

fn add(a: P3, b: P3) -> P3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: P3, k: f64) -> P3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: P3, b: P3) -> P3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn length(a: P3) -> f64 {
    dot(a, a).sqrt()
}
fn normalized(a: P3) -> Option<P3> {
    let l = length(a);
    (l > 1e-300 && l.is_finite()).then(|| scale(a, 1.0 / l))
}
fn lerp(a: P3, b: P3, t: f64) -> P3 {
    add(a, scale(sub(b, a), t))
}

/// Two unit vectors at right angles to `n` and each other, `x` along
/// `reference` where it is not along `n`.
fn frame(n: P3, reference: Option<P3>) -> (P3, P3) {
    let pick = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let r = reference.filter(|r| length(cross(*r, n)) > 1e-9).unwrap_or(pick);
    let x = normalized(sub(r, scale(n, dot(r, n)))).unwrap_or([1.0, 0.0, 0.0]);
    (x, cross(n, x))
}

/// Whether the bytes are an IGES file: every IGES line ends its first 72
/// columns with its section's letter, and the first is the start section's
/// (or a compressed or binary file's flag).
pub fn looks_like(bytes: &[u8]) -> bool {
    let line = bytes.split(|&b| b == b'\n').next().unwrap_or(&[]);
    line.len() >= 73 && matches!(line[72], b'S' | b'C' | b'B')
}

// The file.

/// A parameter: a number, a string, or left out.
#[derive(Clone, Debug, PartialEq)]
enum Value {
    Number(f64),
    Text(String),
    Empty,
}

/// An entity: its directory entry and its parameters, the type number
/// taken off the front.
#[derive(Clone, Debug)]
struct Entity {
    kind: u32,
    form: i64,
    matrix: u64,
    colour: i64,
    blanked: bool,
    dependent: bool,
    label: String,
    params: Vec<Value>,
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// The parameters of a free-format record, split at `delimiter` up to
/// `end`, Hollerith strings (`5Hhello`) read whole.
fn values(data: &[u8], delimiter: u8, end: u8) -> Vec<Value> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        while i < data.len() && data[i] == b' ' {
            i += 1;
        }
        let mut j = i;
        while j < data.len() && data[j].is_ascii_digit() {
            j += 1;
        }
        if j > i && j < data.len() && (data[j] == b'H' || data[j] == b'h') {
            let n: usize = latin1(&data[i..j]).parse().unwrap_or(0);
            let text_end = (j + 1 + n).min(data.len());
            out.push(Value::Text(latin1(&data[j + 1..text_end])));
            let mut k = text_end;
            while k < data.len() && data[k] == b' ' {
                k += 1;
            }
            i = k;
        } else {
            while j < data.len() && data[j] != delimiter && data[j] != end {
                j += 1;
            }
            let token = latin1(&data[i..j]);
            let token = token.trim();
            out.push(if token.is_empty() {
                Value::Empty
            } else {
                match token.replace(['D', 'd'], "E").parse::<f64>() {
                    Ok(v) => Value::Number(v),
                    Err(_) => Value::Text(token.to_string()),
                }
            });
            i = j;
        }
        if i >= data.len() || data[i] == end {
            break;
        }
        // The delimiter.
        i += 1;
    }
    out
}

struct File {
    entities: BTreeMap<u64, Entity>,
    unit: Option<LengthUnit>,
    /// What the file's coordinates are multiplied by to be in `unit`, for
    /// the units Tequila has no name for (kilometres, microns).
    scale: f64,
}

fn parse(bytes: &[u8]) -> Result<File, String> {
    let first = bytes.split(|&b| b == b'\n').next().unwrap_or(&[]);
    match first.get(72) {
        Some(b'C') => return Err("this is compressed IGES, which Tequila does not read; save it again as plain (fixed) IGES".into()),
        Some(b'B') => return Err("this is binary IGES, which Tequila does not read; save it again as plain (fixed) IGES".into()),
        _ => {}
    }
    let mut global: Vec<u8> = Vec::new();
    let mut directory: Vec<&[u8]> = Vec::new();
    let mut parameters: HashMap<u64, &[u8]> = HashMap::new();
    for raw in bytes.split(|&b| b == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        if line.len() < 73 {
            return Err(format!("a line is {} columns long, short of the 73 every IGES line has", line.len()));
        }
        let sequence = || latin1(&line[73..line.len().min(80)]).trim().parse::<u64>().unwrap_or(0);
        match line[72] {
            b'S' | b'T' => {}
            b'G' => global.extend_from_slice(&line[..72]),
            b'D' => directory.push(line),
            b'P' => {
                parameters.insert(sequence(), &line[..64]);
            }
            other => return Err(format!("a line belongs to no IGES section ('{}' in column 73)", other as char)),
        }
    }

    // The global section names its own delimiters first, as Hollerith
    // strings; left out, they are a comma and a semicolon.
    let mut at = 0;
    let mut delimiter = b',';
    let mut end = b';';
    let read_delimiter = |at: &mut usize, fallback: u8, after: u8| -> u8 {
        while *at < global.len() && global[*at] == b' ' {
            *at += 1;
        }
        if global.get(*at..*at + 2).is_some_and(|h| h.eq_ignore_ascii_case(b"1H")) && *at + 2 < global.len() {
            let d = global[*at + 2];
            *at += 3;
            if global.get(*at) == Some(&after) || global.get(*at) == Some(&d) {
                *at += 1;
            }
            d
        } else {
            if global.get(*at) == Some(&after) {
                *at += 1;
            }
            fallback
        }
    };
    delimiter = read_delimiter(&mut at, delimiter, b',');
    end = read_delimiter(&mut at, end, delimiter);
    let fields = values(&global[at.min(global.len())..], delimiter, end);
    // Fields from the third: the unit flag is the 14th, its name the 15th.
    let field = |n: usize| fields.get(n - 3);
    let flag = field(14).and_then(|v| if let Value::Number(n) = v { Some(*n as i64) } else { None }).unwrap_or(0);
    let unit_name = field(15).and_then(|v| if let Value::Text(t) = v { Some(t.trim().to_ascii_uppercase()) } else { None }).unwrap_or_default();
    let (unit, scale) = match (flag, unit_name.as_str()) {
        (1, _) | (3, "IN" | "INCH" | "INCHES") => (Some(LengthUnit::Inch), 1.0),
        (2, _) | (3, "MM" | "MILLIMETER" | "MILLIMETRE") => (Some(LengthUnit::Millimetre), 1.0),
        (4, _) | (3, "FT" | "FOOT" | "FEET") => (Some(LengthUnit::Foot), 1.0),
        (5, _) | (3, "MI" | "MILE" | "MILES") => (Some(LengthUnit::Metre), 1609.344),
        (6, _) | (3, "M" | "METER" | "METRE") => (Some(LengthUnit::Metre), 1.0),
        (7, _) | (3, "KM") => (Some(LengthUnit::Metre), 1000.0),
        (8, _) | (3, "MIL" | "MILS") => (Some(LengthUnit::Millimetre), 0.0254),
        (9, _) | (3, "UM" | "MICRON" | "MICRONS") => (Some(LengthUnit::Millimetre), 0.001),
        (10, _) | (3, "CM") => (Some(LengthUnit::Centimetre), 1.0),
        (11, _) | (3, "UIN") => (Some(LengthUnit::Millimetre), 2.54e-5),
        _ => (None, 1.0),
    };

    let field8 = |line: &[u8], k: usize| latin1(&line[(k * 8).min(line.len())..(k * 8 + 8).min(line.len())]).trim().to_string();
    let int = |s: &str| s.parse::<i64>().unwrap_or(0);
    let mut entities = BTreeMap::new();
    for pair in directory.chunks(2) {
        let [a, b] = pair else { break };
        let de = latin1(&a[73..a.len().min(80)]).trim().parse::<u64>().unwrap_or(0);
        let kind = int(&field8(a, 0)).max(0) as u32;
        let pointer = int(&field8(a, 1)).max(0) as u64;
        let status = format!("{:0>8}", field8(a, 8).replace(' ', "0"));
        let count = int(&field8(b, 3)).max(1) as u64;
        let mut data = Vec::new();
        for sequence in pointer..pointer + count {
            if let Some(line) = parameters.get(&sequence) {
                data.extend_from_slice(line);
            }
        }
        let mut params = values(&data, delimiter, end);
        if !params.is_empty() {
            params.remove(0);
        }
        entities.insert(
            de,
            Entity {
                kind,
                form: int(&field8(b, 4)),
                matrix: int(&field8(a, 6)).max(0) as u64,
                colour: int(&field8(b, 2)),
                blanked: &status[0..2] == "01",
                dependent: &status[2..4] != "00",
                label: field8(b, 7),
                params,
            },
        );
    }
    Ok(File { entities, unit, scale })
}

// Geometry.

/// A map of space: rotation (and scale) then translation.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Affine {
    r: [[f64; 3]; 3],
    t: P3,
}

impl Affine {
    const IDENTITY: Affine = Affine { r: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], t: [0.0; 3] };

    fn apply(&self, p: P3) -> P3 {
        add(self.linear(p), self.t)
    }

    fn linear(&self, v: P3) -> P3 {
        let r = &self.r;
        [dot(r[0], v), dot(r[1], v), dot(r[2], v)]
    }

    /// This map, then `outer`.
    fn then(&self, outer: &Affine) -> Affine {
        let mut r = [[0.0; 3]; 3];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| outer.r[i][k] * self.r[k][j]).sum();
            }
        }
        Affine { r, t: outer.apply(self.t) }
    }

    fn inverse(&self) -> Option<Affine> {
        let m = &self.r;
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        if det.abs() < 1e-300 {
            return None;
        }
        let c = |i: usize, j: usize| {
            let (a, b) = ((i + 1) % 3, (i + 2) % 3);
            let (x, y) = ((j + 1) % 3, (j + 2) % 3);
            m[a][x] * m[b][y] - m[a][y] * m[b][x]
        };
        let mut r = [[0.0; 3]; 3];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = c(j, i) / det;
            }
        }
        let inverse = Affine { r, t: [0.0; 3] };
        Some(Affine { r, t: scale(inverse.linear(self.t), -1.0) })
    }

    fn to_transform(self, unit_scale: f64) -> Transform {
        let mut m = Mat4::IDENTITY;
        for i in 0..3 {
            for j in 0..3 {
                m.m[i][j] = self.r[i][j] as f32;
            }
            m.m[i][3] = (self.t[i] * unit_scale) as f32;
        }
        Transform::from_matrix(m).unwrap_or(Transform::IDENTITY)
    }
}

type CurveFn = Arc<dyn Fn(f64) -> P3 + Send + Sync>;
type SurfaceFn = Arc<dyn Fn(f64, f64) -> P3 + Send + Sync>;
type InvertFn = Arc<dyn Fn(P3) -> P2 + Send + Sync>;

/// A stretch of curve over `t0` to `t1` (either way round), sampled at
/// least `min` times: a NURBS span, an arc, one segment of a polyline.
#[derive(Clone)]
struct Piece {
    at: CurveFn,
    t0: f64,
    t1: f64,
    min: usize,
}

impl Piece {
    fn new(at: impl Fn(f64) -> P3 + Send + Sync + 'static, t0: f64, t1: f64, min: usize) -> Piece {
        Piece { at: Arc::new(at), t0, t1, min }
    }

    fn line(a: P3, b: P3) -> Piece {
        Piece::new(move |t| lerp(a, b, t), 0.0, 1.0, 1)
    }

    fn mapped(self, m: &Affine) -> Piece {
        if *m == Affine::IDENTITY {
            return self;
        }
        let (m, f) = (*m, self.at);
        Piece { at: Arc::new(move |t| m.apply(f(t))), t0: self.t0, t1: self.t1, min: self.min }
    }

    fn reversed(self) -> Piece {
        Piece { t0: self.t1, t1: self.t0, ..self }
    }

    fn start(&self) -> P3 {
        (self.at)(self.t0)
    }

    fn end(&self) -> P3 {
        (self.at)(self.t1)
    }
}

fn reversed(pieces: Vec<Piece>) -> Vec<Piece> {
    pieces.into_iter().rev().map(Piece::reversed).collect()
}

/// Points along a piece, close enough that, carried through `map`, no
/// chord strays from the curve by more than `tolerance`.
fn sample(piece: &Piece, map: &dyn Fn(P3) -> P3, tolerance: f64) -> Vec<P3> {
    fn refine(piece: &Piece, map: &dyn Fn(P3) -> P3, a: f64, b: f64, tolerance: f64, depth: usize, out: &mut Vec<P3>) {
        let middle = (a + b) / 2.0;
        let (pa, pm, pb) = ((piece.at)(a), (piece.at)(middle), (piece.at)(b));
        let (qa, qm, qb) = (map(pa), map(pm), map(pb));
        if depth < 14 && length(sub(qm, lerp(qa, qb, 0.5))) > tolerance {
            refine(piece, map, a, middle, tolerance, depth + 1, out);
            refine(piece, map, middle, b, tolerance, depth + 1, out);
        } else {
            out.push(pb);
        }
    }
    let n = piece.min.max(1);
    let mut out = vec![piece.start()];
    for i in 0..n {
        let a = piece.t0 + (piece.t1 - piece.t0) * i as f64 / n as f64;
        let b = piece.t0 + (piece.t1 - piece.t0) * (i + 1) as f64 / n as f64;
        refine(piece, map, a, b, tolerance, 0, &mut out);
    }
    out
}

/// Pieces end to end as one curve over 0 to 1, each its share of the
/// whole by its length: how ruled surfaces and the like run along a curve.
fn path(pieces: &[Piece]) -> CurveFn {
    let lengths: Vec<f64> = pieces
        .iter()
        .map(|p| {
            (0..16)
                .map(|i| {
                    let (a, b) = (p.t0 + (p.t1 - p.t0) * i as f64 / 16.0, p.t0 + (p.t1 - p.t0) * (i + 1) as f64 / 16.0);
                    length(sub((p.at)(b), (p.at)(a)))
                })
                .sum::<f64>()
                .max(1e-300)
        })
        .collect();
    let total: f64 = lengths.iter().sum();
    let pieces: Vec<Piece> = pieces.to_vec();
    Arc::new(move |s: f64| {
        let mut at = s.clamp(0.0, 1.0) * total;
        for (piece, l) in pieces.iter().zip(&lengths) {
            if at <= *l || std::ptr::eq(piece, pieces.last().unwrap()) {
                let f = (at / l).clamp(0.0, 1.0);
                return (piece.at)(piece.t0 + (piece.t1 - piece.t0) * f);
            }
            at -= l;
        }
        [0.0; 3]
    })
}

/// The basis functions of a B-spline at `t`, in its knot span: the
/// Cox-de Boor recursion (Piegl and Tiller, A2.2).
fn basis(knots: &[f64], degree: usize, count: usize, t: f64) -> (usize, Vec<f64>) {
    let n = count - 1;
    let t = t.clamp(knots[degree], knots[n + 1]);
    let span = if t >= knots[n + 1] {
        let mut s = n;
        while s > degree && knots[s] >= knots[n + 1] {
            s -= 1;
        }
        s
    } else {
        let (mut low, mut high) = (degree, n + 1);
        let mut mid = (low + high) / 2;
        while t < knots[mid] || t >= knots[mid + 1] {
            if t < knots[mid] {
                high = mid;
            } else {
                low = mid;
            }
            mid = (low + high) / 2;
            if high - low <= 1 {
                mid = low;
                break;
            }
        }
        mid
    };
    let mut b = vec![0.0; degree + 1];
    let (mut left, mut right) = (vec![0.0; degree + 1], vec![0.0; degree + 1]);
    b[0] = 1.0;
    for j in 1..=degree {
        left[j] = t - knots[span + 1 - j];
        right[j] = knots[span + j] - t;
        let mut saved = 0.0;
        for r in 0..j {
            let denominator = right[r + 1] + left[j - r];
            let temp = if denominator != 0.0 { b[r] / denominator } else { 0.0 };
            b[r] = saved + right[r + 1] * temp;
            saved = left[j - r] * temp;
        }
        b[j] = saved;
    }
    (span, b)
}

/// A rational B-spline curve from `t0` to `t1` as pieces, one a knot span:
/// its points homogeneous, already multiplied by their weights.
fn nurbs_pieces(knots: Vec<f64>, degree: usize, points: Vec<[f64; 4]>, t0: f64, t1: f64) -> Vec<Piece> {
    let curve = Arc::new((knots.clone(), points));
    spans(&knots, t0, t1)
        .into_iter()
        .map(|(a, b)| {
            let curve = curve.clone();
            Piece::new(
                move |t| {
                    let (knots, points) = &*curve;
                    let (span, weights) = basis(knots, degree, points.len(), t);
                    let mut h = [0.0; 4];
                    for (j, w) in weights.iter().enumerate() {
                        let p = points[span - degree + j];
                        for c in 0..4 {
                            h[c] += w * p[c];
                        }
                    }
                    [h[0] / h[3], h[1] / h[3], h[2] / h[3]]
                },
                a,
                b,
                degree.max(1) * 2 - 1,
            )
        })
        .collect()
}

/// A rational B-spline surface, its points homogeneous and multiplied by
/// their weights, u varying fastest: IGES's 128 and KeyShot's faces alike.
fn nurbs_surface(ku: Vec<f64>, kv: Vec<f64>, (d1, d2): (usize, usize), (c1, c2): (usize, usize), points: Vec<[f64; 4]>, domain: [f64; 4]) -> Surface {
    let span_counts = [spans(&ku, domain[0], domain[1]).len(), spans(&kv, domain[2], domain[3]).len()];
    let data = Arc::new((ku, kv, points));
    Surface {
        at: Arc::new(move |u, v| {
            let (ku, kv, points) = &*data;
            let (su, bu) = basis(ku, d1, c1, u);
            let (sv, bv) = basis(kv, d2, c2, v);
            let mut h = [0.0; 4];
            for (j, wv) in bv.iter().enumerate() {
                for (i, wu) in bu.iter().enumerate() {
                    let p = points[(su - d1 + i) + (sv - d2 + j) * c1];
                    let w = wu * wv;
                    for c in 0..4 {
                        h[c] += w * p[c];
                    }
                }
            }
            [h[0] / h[3], h[1] / h[3], h[2] / h[3]]
        }),
        domain,
        spans: [span_counts[0] * d1.max(1), span_counts[1] * d2.max(1)],
        invert: None,
        period: [None, None],
        native: true,
    }
}

/// A trimmed NURBS face given as numbers rather than as IGES entities: what
/// KeyShot's scenes store their faces as (`crate::bip_scene`).
pub(crate) struct NurbsFace {
    pub degree: [usize; 2],
    pub knots: [Vec<f64>; 2],
    /// `[x·w, y·w, z·w, w]`, u varying fastest.
    pub points: Vec<[f64; 4]>,
    /// u0, u1, v0, v1.
    pub domain: [f64; 4],
    /// Its loops, outer first, each of curves end to end in the surface's
    /// parameters; none for a face that is its whole surface.
    pub loops: Vec<Vec<NurbsCurve>>,
}

/// A trimming curve in a surface's parameters.
pub(crate) struct NurbsCurve {
    pub degree: usize,
    pub knots: Vec<f64>,
    /// `[u·w, v·w, 0, w]`.
    pub points: Vec<[f64; 4]>,
    pub range: [f64; 2],
}

impl NurbsCurve {
    /// Whether the numbers hold together: a knot for every point and order,
    /// rising, and the range within them. A curve that does not would index
    /// past its points.
    fn sound(&self) -> bool {
        sound_spline(&self.knots, self.degree, self.points.len())
            && self.range.iter().all(|t| t.is_finite())
            && self.range[1] > self.range[0]
            && self.points.iter().all(|p| p.iter().all(|c| c.is_finite()) && p[3] > 0.0)
    }
}

fn sound_spline(knots: &[f64], degree: usize, count: usize) -> bool {
    degree >= 1 && count > degree && knots.len() == count + degree + 1 && knots.iter().all(|k| k.is_finite()) && knots.windows(2).all(|w| w[1] >= w[0])
}

/// A divided face: points and normals in the surface's space, triangles,
/// and its edges as pairs of end points.
pub(crate) struct Divided {
    pub positions: Vec<[f64; 3]>,
    pub normals: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    pub edges: Vec<[f64; 3]>,
    /// Each point's surface parameters, for the face's own UVs.
    pub params: Vec<[f64; 2]>,
}

/// Divide a trimmed NURBS face at `tolerance`, as IGES's own faces are:
/// constrained Delaunay over its loops and a grid as fine as its bend asks,
/// normals from the surface, and its boundary as edges. `None` for numbers
/// that do not hold together, or a face that came out empty.
pub(crate) fn divide_nurbs(face: NurbsFace, tolerance: f64) -> Option<Divided> {
    let NurbsFace { degree, knots, points, domain, loops } = face;
    let count = [knots[0].len().checked_sub(degree[0] + 1)?, knots[1].len().checked_sub(degree[1] + 1)?];
    if !sound_spline(&knots[0], degree[0], count[0]) || !sound_spline(&knots[1], degree[1], count[1]) || points.len() != count[0] * count[1] {
        return None;
    }
    if !points.iter().all(|p| p.iter().all(|c| c.is_finite()) && p[3] > 0.0) {
        return None;
    }
    let inside = |k: &[f64], d: usize, n: usize, a: f64, b: f64| a.is_finite() && b.is_finite() && b > a && a >= k[d] - 1e-9 * (1.0 + a.abs()) && b <= k[n] + 1e-9 * (1.0 + b.abs());
    if !inside(&knots[0], degree[0], count[0], domain[0], domain[1]) || !inside(&knots[1], degree[1], count[1], domain[2], domain[3]) || !(tolerance > 0.0 && tolerance.is_finite()) {
        return None;
    }
    let [ku, kv] = knots;
    let surface = nurbs_surface(ku, kv, (degree[0], degree[1]), (count[0], count[1]), points, domain);
    let mut segments = Vec::with_capacity(loops.len());
    for l in loops {
        if !l.iter().all(NurbsCurve::sound) {
            return None;
        }
        segments.push(l.into_iter().map(|c| Segment::Params(nurbs_pieces(c.knots, c.degree, c.points, c.range[0], c.range[1]))).collect());
    }
    let face = Face { surface, loops: segments, reversed: false, colour: None, body: None, name: None };
    let meshed = tessellate(&face, tolerance);
    if meshed.triangles.is_empty() {
        return None;
    }
    Some(Divided { positions: meshed.positions, normals: meshed.normals, triangles: meshed.triangles, edges: meshed.edges, params: meshed.params })
}

/// The distinct knot spans of a B-spline between `t0` and `t1`.
fn spans(knots: &[f64], t0: f64, t1: f64) -> Vec<(f64, f64)> {
    let mut breaks: Vec<f64> = knots.iter().copied().filter(|k| *k > t0 && *k < t1).collect();
    breaks.insert(0, t0);
    breaks.push(t1);
    breaks.dedup_by(|a, b| (*a - *b).abs() <= 1e-12 * (1.0 + b.abs()));
    breaks.windows(2).map(|w| (w[0], w[1])).collect()
}

/// A surface, for dividing a face on it.
#[derive(Clone)]
struct Surface {
    at: SurfaceFn,
    /// Parameter ranges: u0, u1, v0, v1. Infinite where the surface goes
    /// on (a plane, a cylinder's length).
    domain: [f64; 4],
    /// Grid divisions its shape needs at least over its whole domain: a
    /// NURBS span each way.
    spans: [usize; 2],
    /// A point's parameters, where the surface has a formula for them.
    invert: Option<InvertFn>,
    /// Parameters a whole turn apart that are the same point.
    period: [Option<f64>; 2],
    /// Whether IGES defines the surface's parameters (NURBS), so curves
    /// given in them (142's B) can be used as they are. The others are
    /// parameterized here, and trimmed by their model-space curves.
    native: bool,
}

impl Surface {
    fn point(&self, uv: P2) -> P3 {
        (self.at)(uv[0], uv[1])
    }

    /// The point and its partial derivatives, by differences, one-sided at
    /// the edge of a bounded domain.
    fn derivatives(&self, uv: P2) -> (P3, P3, P3) {
        let p = self.point(uv);
        let mut d = [[0.0; 3]; 2];
        for k in 0..2 {
            let (lo, hi) = (self.domain[2 * k], self.domain[2 * k + 1]);
            let h = if lo.is_finite() && hi.is_finite() { (hi - lo).abs().max(1e-12) * 1e-6 } else { uv[k].abs().max(1.0) * 1e-7 };
            let shifted = |by: f64| {
                let mut q = uv;
                q[k] += by;
                self.point(q)
            };
            d[k] = if uv[k] + h > hi {
                scale(sub(p, shifted(-h)), 1.0 / h)
            } else if uv[k] - h < lo {
                scale(sub(shifted(h), p), 1.0 / h)
            } else {
                scale(sub(shifted(h), shifted(-h)), 0.5 / h)
            };
        }
        (p, d[0], d[1])
    }

    fn mapped(self, m: &Affine) -> Surface {
        if *m == Affine::IDENTITY {
            return self;
        }
        let (m, f) = (*m, self.at);
        let invert = match (self.invert, m.inverse()) {
            (Some(g), Some(inverse)) => Some(Arc::new(move |p: P3| g(inverse.apply(p))) as InvertFn),
            _ => None,
        };
        Surface { at: Arc::new(move |u, v| m.apply(f(u, v))), invert, ..self }
    }

    fn bounded(&self) -> bool {
        self.domain.iter().all(|d| d.is_finite())
    }

    /// A grid of points over a bounded domain, to start a search for a
    /// point's parameters from the nearest.
    fn seeds(&self) -> Vec<(P2, P3)> {
        if !self.bounded() {
            return Vec::new();
        }
        let [u0, u1, v0, v1] = self.domain;
        let n = 24;
        let mut out = Vec::with_capacity((n + 1) * (n + 1));
        for i in 0..=n {
            for j in 0..=n {
                let uv = [u0 + (u1 - u0) * i as f64 / n as f64, v0 + (v1 - v0) * j as f64 / n as f64];
                out.push((uv, self.point(uv)));
            }
        }
        out
    }

    /// The parameters of the point on the surface nearest `p`: a formula
    /// where there is one, else Newton's method from `hint` (the previous
    /// point along a curve) or the nearest seed.
    fn parameters(&self, p: P3, hint: Option<P2>, seeds: &[(P2, P3)], tolerance: f64) -> P2 {
        if let Some(invert) = &self.invert {
            return invert(p);
        }
        let newton = |mut uv: P2| -> (P2, f64) {
            let [u0, u1, v0, v1] = self.domain;
            for _ in 0..32 {
                let (q, su, sv) = self.derivatives(uv);
                let r = sub(q, p);
                let (a, b, c) = (dot(su, su), dot(su, sv), dot(sv, sv));
                let (e, f) = (-dot(r, su), -dot(r, sv));
                let det = a * c - b * b;
                if det.abs() <= 1e-300 {
                    break;
                }
                let (du, dv) = ((e * c - b * f) / det, (a * f - b * e) / det);
                let next = [(uv[0] + du).clamp(u0.min(u1), u0.max(u1)), (uv[1] + dv).clamp(v0.min(v1), v0.max(v1))];
                let moved = (next[0] - uv[0]).abs() + (next[1] - uv[1]).abs();
                uv = next;
                if moved <= 1e-13 * ((u1 - u0).abs() + (v1 - v0).abs()) {
                    break;
                }
            }
            (uv, length(sub(self.point(uv), p)))
        };
        if let Some(hint) = hint {
            let (uv, miss) = newton(hint);
            if miss <= tolerance {
                return uv;
            }
        }
        let seed = seeds
            .iter()
            .min_by(|a, b| length(sub(a.1, p)).total_cmp(&length(sub(b.1, p))))
            .map(|s| s.0)
            .unwrap_or([self.domain[0], self.domain[2]]);
        newton(seed).0
    }
}

/// A trimming curve, as the file gives it.
#[derive(Clone)]
enum Segment {
    /// In the surface's own parameters (u, v, 0).
    Params(Vec<Piece>),
    /// In space, to be found on the surface.
    Model(Vec<Piece>),
}

/// A face to divide.
#[derive(Clone)]
struct Face {
    surface: Surface,
    /// Its loops, each of segments end to end; none for a surface whose
    /// whole domain is the face.
    loops: Vec<Vec<Segment>>,
    reversed: bool,
    colour: Option<[f32; 3]>,
    /// The solid it is a face of, where the file says.
    body: Option<u64>,
    name: Option<String>,
}

impl Face {
    /// Points on the face's boundary, in space: its corners, to gather
    /// faces that meet into bodies, and to size it.
    fn corners(&self) -> Vec<P3> {
        let mut out = Vec::new();
        for segment in self.loops.iter().flatten() {
            match segment {
                Segment::Params(pieces) => out.extend(pieces.iter().map(|p| {
                    let q = p.start();
                    self.surface.point([q[0], q[1]])
                })),
                Segment::Model(pieces) => out.extend(pieces.iter().map(Piece::start)),
            }
        }
        if self.loops.is_empty() && self.surface.bounded() {
            let [u0, u1, v0, v1] = self.surface.domain;
            for uv in [[u0, v0], [u1, v0], [u1, v1], [u0, v1], [(u0 + u1) / 2.0, (v0 + v1) / 2.0]] {
                out.push(self.surface.point(uv));
            }
        }
        out
    }
}

/// A divided face: points, normals, triangles, and its edges as segments.
#[derive(Default)]
struct Meshed {
    positions: Vec<P3>,
    normals: Vec<P3>,
    triangles: Vec<[u32; 3]>,
    edges: Vec<P3>,
    /// Each point's surface parameters.
    params: Vec<P2>,
}

/// Loops that go once round a closed surface (a cylinder's two end
/// circles) bound a band, not a region of their own: each is cut where the
/// first starts its turn and joined to the next, round the band's edge.
fn close_bands(loops: &mut Vec<Vec<P2>>, period: f64, k: usize, domain: [f64; 4]) {
    let turns = |l: &[P2]| l.len() > 1 && (l[l.len() - 1][k] - l[0][k]).abs() > period / 2.0;
    let wrapping: Vec<usize> = (0..loops.len()).filter(|&i| turns(&loops[i])).collect();
    if wrapping.is_empty() {
        return;
    }
    let cut = loops[wrapping[0]][0][k];
    // Each turning loop rewritten to run from `cut` to `cut + period`.
    let mut runs: Vec<Vec<P2>> = Vec::new();
    for &i in &wrapping {
        let mut l = loops[i].clone();
        if l[l.len() - 1][k] < l[0][k] {
            l.reverse();
        }
        // Unrolled twice, then the stretch from `cut` to `cut + period`.
        let shift = ((cut - l[0][k]) / period).floor() * period;
        let mut unrolled: Vec<P2> = Vec::new();
        for turn in 0..3 {
            for p in &l {
                let mut q = *p;
                q[k] += shift + period * turn as f64;
                unrolled.push(q);
            }
        }
        let at = |value: f64| -> Option<P2> {
            unrolled.windows(2).find(|w| (w[0][k] - value) * (w[1][k] - value) <= 0.0 && w[0][k] != w[1][k]).map(|w| {
                let f = (value - w[0][k]) / (w[1][k] - w[0][k]);
                let mut p = [w[0][0] + (w[1][0] - w[0][0]) * f, w[0][1] + (w[1][1] - w[0][1]) * f];
                p[k] = value;
                p
            })
        };
        let (Some(start), Some(finish)) = (at(cut), at(cut + period)) else { continue };
        let mut run = vec![start];
        run.extend(unrolled.iter().copied().filter(|p| p[k] > cut && p[k] < cut + period));
        run.push(finish);
        runs.push(run);
    }
    // Paired by where they sit across the band; an odd one out closes on
    // the nearer edge of the domain (a cone's apex, a sphere's pole).
    let other = 1 - k;
    let mean = |l: &[P2]| l.iter().map(|p| p[other]).sum::<f64>() / l.len() as f64;
    runs.sort_by(|a, b| mean(a).total_cmp(&mean(b)));
    let mut closed = Vec::new();
    let mut i = 0;
    while i < runs.len() {
        let mut region = runs[i].clone();
        if i + 1 < runs.len() {
            region.extend(runs[i + 1].iter().rev());
            i += 2;
        } else {
            let (lo, hi) = (domain[2 * other], domain[2 * other + 1]);
            let m = mean(&runs[i]);
            let edge = if (m - lo).abs() < (hi - m).abs() { lo } else { hi };
            if !edge.is_finite() {
                i += 1;
                continue;
            }
            let mut a = region[region.len() - 1];
            let mut b = region[0];
            a[other] = edge;
            b[other] = edge;
            region.push(a);
            region.push(b);
            i += 1;
        }
        closed.push(region);
    }
    let mut keep: Vec<Vec<P2>> = (0..loops.len()).filter(|i| !wrapping.contains(i)).map(|i| loops[i].clone()).collect();
    // The other loops brought onto the same turn.
    for l in &mut keep {
        let m = l.iter().map(|p| p[k]).sum::<f64>() / l.len() as f64;
        let shift = ((cut - m) / period).ceil() * period;
        for p in l.iter_mut() {
            p[k] += shift;
        }
    }
    keep.extend(closed);
    *loops = keep;
}

/// Divide a face at `tolerance`.
fn tessellate(face: &Face, tolerance: f64) -> Meshed {
    let s = &face.surface;
    let seeds = if s.invert.is_none() && face.loops.iter().flatten().any(|g| matches!(g, Segment::Model(_))) { s.seeds() } else { Vec::new() };
    let mut loops: Vec<Vec<P2>> = Vec::new();
    // Each piece of boundary in space, for the edges.
    let mut boundary: Vec<Vec<P3>> = Vec::new();
    let surface_map = |q: P3| s.point([q[0], q[1]]);
    let add_loop = |segments: &[Segment], loops: &mut Vec<Vec<P2>>, boundary: &mut Vec<Vec<P3>>| {
        let mut uv: Vec<P2> = Vec::new();
        for segment in segments {
            match segment {
                Segment::Params(pieces) => {
                    for piece in pieces {
                        let points = sample(piece, &surface_map, tolerance);
                        boundary.push(points.iter().map(|q| s.point([q[0], q[1]])).collect());
                        uv.extend(points.iter().map(|q| [q[0], q[1]]));
                    }
                }
                Segment::Model(pieces) => {
                    for piece in pieces {
                        let points = sample(piece, &|p| p, tolerance);
                        for p in &points {
                            let hint = uv.last().copied();
                            let mut found = s.parameters(*p, hint, &seeds, tolerance);
                            // A turning parameter found anew each time
                            // is followed on from the last, not reset to
                            // its first turn.
                            if let Some(last) = hint {
                                for k in 0..2 {
                                    if let Some(period) = s.period[k] {
                                        found[k] -= ((found[k] - last[k]) / period).round() * period;
                                    }
                                }
                            }
                            uv.push(found);
                        }
                        boundary.push(points);
                    }
                }
            }
        }
        loops.push(uv);
    };
    if face.loops.is_empty() {
        if !s.bounded() {
            return Meshed::default();
        }
        let [u0, u1, v0, v1] = s.domain;
        let side = |a: P2, b: P2| Piece::line([a[0], a[1], 0.0], [b[0], b[1], 0.0]);
        let sides = vec![Segment::Params(vec![side([u0, v0], [u1, v0]), side([u1, v0], [u1, v1]), side([u1, v1], [u0, v1]), side([u0, v1], [u0, v0])])];
        add_loop(&sides, &mut loops, &mut boundary);
    } else {
        for l in &face.loops {
            add_loop(l, &mut loops, &mut boundary);
        }
    }
    for k in 0..2 {
        if let Some(period) = s.period[k] {
            close_bands(&mut loops, period, k, s.domain);
        }
    }
    loops.retain(|l| l.len() >= 3);
    if loops.is_empty() {
        return Meshed::default();
    }

    // The grid: over the loops' extent, as fine as the surface's bend asks.
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in loops.iter().flatten() {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let extent = [hi[0] - lo[0], hi[1] - lo[1]];
    if !(extent[0] > 0.0 && extent[1] > 0.0) {
        return Meshed::default();
    }
    let share = |k: usize| {
        let (d0, d1) = (s.domain[2 * k], s.domain[2 * k + 1]);
        if d0.is_finite() && d1.is_finite() && d1 > d0 { (s.spans[k] as f64 * extent[k] / (d1 - d0)).ceil().max(1.0) } else { 1.0 }
    };
    let coarse = [((share(0) * 2.0) as usize).clamp(4, 48), ((share(1) * 2.0) as usize).clamp(4, 48)];
    let grid: Vec<Vec<P3>> = (0..=coarse[0])
        .map(|i| (0..=coarse[1]).map(|j| s.point([lo[0] + extent[0] * i as f64 / coarse[0] as f64, lo[1] + extent[1] * j as f64 / coarse[1] as f64])).collect())
        .collect();
    let (mut bend, mut run) = ([0.0f64; 2], [0.0f64; 2]);
    for i in 0..=coarse[0] {
        for j in 0..=coarse[1] {
            if i > 0 && i < coarse[0] {
                bend[0] = bend[0].max(length(sub(add(grid[i - 1][j], grid[i + 1][j]), scale(grid[i][j], 2.0))));
            }
            if j > 0 && j < coarse[1] {
                bend[1] = bend[1].max(length(sub(add(grid[i][j - 1], grid[i][j + 1]), scale(grid[i][j], 2.0))));
            }
            if i > 0 {
                run[0] += length(sub(grid[i][j], grid[i - 1][j])) / (coarse[1] + 1) as f64;
            }
            if j > 0 {
                run[1] += length(sub(grid[i][j], grid[i][j - 1])) / (coarse[0] + 1) as f64;
            }
        }
    }
    // A chord over a grid step strays by about an eighth of the second
    // difference there, which falls with the square of the step. A cell's
    // diagonal bends both ways, so each way gets half the tolerance.
    let flat = bend[0] <= tolerance * 1e-3 && bend[1] <= tolerance * 1e-3;
    let mut steps = [0usize; 2];
    for k in 0..2 {
        let finer = (bend[k] / (4.0 * tolerance)).sqrt().max(1.0);
        steps[k] = ((coarse[k] as f64 * finer).ceil() as usize).clamp(1, 600);
    }
    while steps[0] * steps[1] > 200_000 {
        steps = [(steps[0] * 3 / 4).max(1), (steps[1] * 3 / 4).max(1)];
    }
    // Parameters scaled to lengths, so that Delaunay's triangles are
    // well shaped on the surface rather than in the parameters.
    let stretch = [(run[0] / extent[0]).max(1e-300), (run[1] / extent[1]).max(1e-300)];
    let scaled: Vec<Vec<P2>> = loops.iter().map(|l| l.iter().map(|p| [p[0] * stretch[0], p[1] * stretch[1]]).collect()).collect();
    let mut extra = Vec::new();
    if !flat {
        let cell = [extent[0] * stretch[0] / steps[0] as f64, extent[1] * stretch[1] / steps[1] as f64];
        // Rows and columns crossing the loops: inside by parity, and kept
        // a third of a cell clear of them.
        let crossings = |k: usize, value: f64| -> Vec<f64> {
            let other = 1 - k;
            let mut out = Vec::new();
            for l in &scaled {
                for i in 0..l.len() {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    if (a[k] > value) != (b[k] > value) {
                        out.push(a[other] + (b[other] - a[other]) * (value - a[k]) / (b[k] - a[k]));
                    }
                }
            }
            out.sort_by(f64::total_cmp);
            out
        };
        let clear = |list: &[f64], x: f64, margin: f64| -> (bool, bool) {
            let below = list.partition_point(|c| *c < x);
            let near = [below.checked_sub(1).map(|i| list[i]), list.get(below).copied()].into_iter().flatten().any(|c| (c - x).abs() < margin);
            (below % 2 == 1, !near)
        };
        let columns: Vec<Vec<f64>> = (1..steps[0]).map(|i| crossings(0, (lo[0] + extent[0] * i as f64 / steps[0] as f64) * stretch[0])).collect();
        for j in 1..steps[1] {
            let y = (lo[1] + extent[1] * j as f64 / steps[1] as f64) * stretch[1];
            let row = crossings(1, y);
            for i in 1..steps[0] {
                let x = (lo[0] + extent[0] * i as f64 / steps[0] as f64) * stretch[0];
                let (inside, clear_row) = clear(&row, x, cell[0] / 3.0);
                let (_, clear_column) = clear(&columns[i - 1], y, cell[1] / 3.0);
                if inside && clear_row && clear_column {
                    extra.push([x, y]);
                }
            }
        }
    }
    let (points, triangles) = crate::planar::triangulate(&scaled, &extra);

    let mut meshed = Meshed::default();
    for q in &points {
        let uv = [q[0] / stretch[0], q[1] / stretch[1]];
        let (p, su, sv) = s.derivatives(uv);
        meshed.positions.push(p);
        meshed.params.push(uv);
        let n = normalized(cross(su, sv)).unwrap_or([0.0; 3]);
        meshed.normals.push(if face.reversed { scale(n, -1.0) } else { n });
    }
    for t in triangles {
        let t = t.map(|i| i as u32);
        meshed.triangles.push(if face.reversed { [t[0], t[2], t[1]] } else { t });
    }
    // Where the surface has no normal (a pole), the triangles' own.
    let mut fallback = vec![[0.0; 3]; meshed.positions.len()];
    for t in &meshed.triangles {
        let [a, b, c] = t.map(|i| meshed.positions[i as usize]);
        let n = cross(sub(b, a), sub(c, a));
        for i in t {
            fallback[*i as usize] = add(fallback[*i as usize], n);
        }
    }
    for (n, f) in meshed.normals.iter_mut().zip(&fallback) {
        if *n == [0.0; 3] {
            *n = normalized(*f).unwrap_or([0.0, 0.0, 1.0]);
        }
    }

    // Edges: each boundary piece, but not a seam (a piece the face runs
    // along twice, once each way) or a pole (a piece of no length).
    let near = |a: P3, b: P3| length(sub(a, b)) <= tolerance * 10.0;
    // Its two ends and the point halfway along it.
    let ends = |p: &Vec<P3>| {
        let total: f64 = p.windows(2).map(|w| length(sub(w[1], w[0]))).sum();
        let mut left = total / 2.0;
        let mut halfway = p[p.len() - 1];
        for w in p.windows(2) {
            let l = length(sub(w[1], w[0]));
            if l >= left && l > 0.0 {
                halfway = lerp(w[0], w[1], left / l);
                break;
            }
            left -= l;
        }
        (p[0], halfway, p[p.len() - 1])
    };
    for (i, piece) in boundary.iter().enumerate() {
        if piece.len() < 2 {
            continue;
        }
        let (a, m, b) = ends(piece);
        let span: f64 = piece.windows(2).map(|w| length(sub(w[1], w[0]))).sum();
        if span <= tolerance * 10.0 {
            continue;
        }
        let seam = boundary.iter().enumerate().any(|(j, other)| {
            if j == i || other.len() < 2 {
                return false;
            }
            let (c, n, d) = ends(other);
            near(a, d) && near(b, c) && near(m, n)
        });
        if !seam {
            for w in piece.windows(2) {
                meshed.edges.push(w[0]);
                meshed.edges.push(w[1]);
            }
        }
    }
    meshed
}

// Reading the entities.

struct Reader {
    entities: BTreeMap<u64, Entity>,
    /// Kinds of entity met and not read, with how often.
    skipped: RefCell<BTreeMap<u32, usize>>,
}

impl Reader {
    fn get(&self, de: u64) -> Option<&Entity> {
        self.entities.get(&de)
    }

    fn skip(&self, kind: u32) {
        *self.skipped.borrow_mut().entry(kind).or_insert(0) += 1;
    }

    fn number(e: &Entity, i: usize) -> f64 {
        match e.params.get(i) {
            Some(Value::Number(n)) => *n,
            _ => 0.0,
        }
    }

    fn int(e: &Entity, i: usize) -> i64 {
        Reader::number(e, i).round() as i64
    }

    fn pointer(e: &Entity, i: usize) -> u64 {
        Reader::int(e, i).unsigned_abs()
    }

    fn text(e: &Entity, i: usize) -> Option<String> {
        match e.params.get(i) {
            Some(Value::Text(t)) => Some(t.trim().to_string()).filter(|t| !t.is_empty()),
            _ => None,
        }
    }

    fn triple(e: &Entity, i: usize) -> P3 {
        [Reader::number(e, i), Reader::number(e, i + 1), Reader::number(e, i + 2)]
    }

    /// A transformation matrix (124) and those it is itself placed by.
    fn matrix(&self, de: u64, depth: usize) -> Affine {
        let Some(e) = self.get(de).filter(|e| e.kind == 124) else { return Affine::IDENTITY };
        let n = |i| Reader::number(e, i);
        let own = Affine { r: [[n(0), n(1), n(2)], [n(4), n(5), n(6)], [n(8), n(9), n(10)]], t: [n(3), n(7), n(11)] };
        if e.matrix != 0 && e.matrix != de && depth < 16 { own.then(&self.matrix(e.matrix, depth + 1)) } else { own }
    }

    fn placement(&self, e: &Entity) -> Affine {
        if e.matrix == 0 { Affine::IDENTITY } else { self.matrix(e.matrix, 0) }
    }

    /// A point entity (116).
    fn point(&self, de: u64) -> Option<P3> {
        let e = self.get(de).filter(|e| e.kind == 116)?;
        Some(self.placement(e).apply(Reader::triple(e, 0)))
    }

    /// A direction entity (123).
    fn direction(&self, de: u64) -> Option<P3> {
        let e = self.get(de).filter(|e| e.kind == 123)?;
        normalized(self.placement(e).linear(Reader::triple(e, 0)))
    }

    /// A curve, in its parent's space.
    fn curve(&self, de: u64, depth: usize) -> Option<Vec<Piece>> {
        let e = self.get(de)?;
        if depth > 16 {
            return None;
        }
        let n = |i| Reader::number(e, i);
        let pieces: Vec<Piece> = match e.kind {
            110 => vec![Piece::line(Reader::triple(e, 0), Reader::triple(e, 3))],
            100 => {
                let (z, c, start, end) = (n(0), [n(1), n(2)], [n(3), n(4)], [n(5), n(6)]);
                let radius = ((start[0] - c[0]).powi(2) + (start[1] - c[1]).powi(2)).sqrt();
                let a0 = (start[1] - c[1]).atan2(start[0] - c[0]);
                let mut a1 = (end[1] - c[1]).atan2(end[0] - c[0]);
                // Counter-clockwise from start to end; a whole circle where
                // they meet.
                while a1 <= a0 + 1e-12 {
                    a1 += TAU;
                }
                let segments = ((a1 - a0) / (PI / 8.0)).ceil().max(2.0) as usize;
                vec![Piece::new(move |t| [c[0] + radius * t.cos(), c[1] + radius * t.sin(), z], a0, a1, segments)]
            }
            104 => {
                // An ellipse; hyperbolas and parabolas are not read.
                let [a, b, c, d, f_e, f] = [n(0), n(1), n(2), n(3), n(4), n(5)];
                if 4.0 * a * c - b * b <= 0.0 {
                    self.skip(104);
                    return None;
                }
                let z = n(6);
                let det = 4.0 * a * c - b * b;
                let centre = [(b * f_e - 2.0 * c * d) / det, (b * d - 2.0 * a * f_e) / det];
                let at_centre = a * centre[0] * centre[0] + b * centre[0] * centre[1] + c * centre[1] * centre[1] + d * centre[0] + f_e * centre[1] + f;
                let angle = 0.5 * b.atan2(a - c);
                let (cs, sn) = (angle.cos(), angle.sin());
                let l1 = a * cs * cs + b * cs * sn + c * sn * sn;
                let l2 = a * sn * sn - b * cs * sn + c * cs * cs;
                if at_centre * l1 >= 0.0 || at_centre * l2 >= 0.0 {
                    self.skip(104);
                    return None;
                }
                let (ra, rb) = ((-at_centre / l1).sqrt(), (-at_centre / l2).sqrt());
                let param = |x: f64, y: f64| {
                    let (dx, dy) = (x - centre[0], y - centre[1]);
                    ((-sn * dx + cs * dy) / rb).atan2((cs * dx + sn * dy) / ra)
                };
                let t0 = param(n(7), n(8));
                let mut t1 = param(n(9), n(10));
                while t1 <= t0 + 1e-12 {
                    t1 += TAU;
                }
                let segments = ((t1 - t0) / (PI / 8.0)).ceil().max(2.0) as usize;
                vec![Piece::new(
                    move |t| {
                        let (x, y) = (ra * t.cos(), rb * t.sin());
                        [centre[0] + cs * x - sn * y, centre[1] + sn * x + cs * y, z]
                    },
                    t0,
                    t1,
                    segments,
                )]
            }
            106 => {
                let (kind, count) = (Reader::int(e, 0), Reader::int(e, 1).max(0) as usize);
                let points: Vec<P3> = match kind {
                    1 => (0..count).map(|i| [n(3 + 2 * i), n(4 + 2 * i), n(2)]).collect(),
                    2 => (0..count).map(|i| Reader::triple(e, 2 + 3 * i)).collect(),
                    3 => (0..count).map(|i| Reader::triple(e, 2 + 6 * i)).collect(),
                    _ => Vec::new(),
                };
                if !matches!(e.form, 1..=3 | 11..=13 | 63) || points.len() < 2 {
                    self.skip(106);
                    return None;
                }
                let mut pieces: Vec<Piece> = points.windows(2).map(|w| Piece::line(w[0], w[1])).collect();
                if e.form == 63 && points[0] != points[points.len() - 1] {
                    pieces.push(Piece::line(points[points.len() - 1], points[0]));
                }
                pieces
            }
            102 => {
                let count = Reader::int(e, 0).max(0) as usize;
                let mut pieces = Vec::new();
                for i in 0..count {
                    pieces.extend(self.curve(Reader::pointer(e, 1 + i), depth + 1)?);
                }
                pieces
            }
            112 => {
                let segments = Reader::int(e, 3).max(0) as usize;
                let breaks: Vec<f64> = (0..=segments).map(|i| n(4 + i)).collect();
                (0..segments)
                    .map(|i| {
                        let base = 5 + segments + 12 * i;
                        let c: Vec<f64> = (0..12).map(|k| n(base + k)).collect();
                        let cubic = move |s: f64| {
                            let at = |o: usize| c[o] + s * (c[o + 1] + s * (c[o + 2] + s * c[o + 3]));
                            [at(0), at(4), at(8)]
                        };
                        Piece::new(cubic, 0.0, breaks[i + 1] - breaks[i], 4)
                    })
                    .collect()
            }
            126 => {
                let (k, degree) = (Reader::int(e, 0).max(0) as usize, Reader::int(e, 1).max(0) as usize);
                let count = k + 1;
                let knots: Vec<f64> = (0..k + degree + 2).map(|i| n(6 + i)).collect();
                let weights: Vec<f64> = (0..count).map(|i| n(6 + knots.len() + i)).map(|w| if w > 0.0 { w } else { 1.0 }).collect();
                let base = 6 + knots.len() + count;
                let points: Vec<[f64; 4]> = (0..count)
                    .map(|i| {
                        let p = Reader::triple(e, base + 3 * i);
                        [p[0] * weights[i], p[1] * weights[i], p[2] * weights[i], weights[i]]
                    })
                    .collect();
                let (v0, v1) = (n(base + 3 * count), n(base + 3 * count + 1));
                if degree == 0 || knots.len() < degree + 2 || !(v1 > v0) {
                    self.skip(126);
                    return None;
                }
                nurbs_pieces(knots, degree, points, v0, v1)
            }
            other => {
                self.skip(other);
                return None;
            }
        };
        let placement = self.placement(e);
        Some(pieces.into_iter().map(|p| p.mapped(&placement)).collect())
    }

    /// A surface, in its parent's space.
    fn surface(&self, de: u64, depth: usize) -> Option<Surface> {
        let e = self.get(de)?;
        let n = |i| Reader::number(e, i);
        let infinite = [f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY];
        let plane = |origin: P3, normal: P3, reference: Option<P3>| -> Surface {
            let (x, y) = frame(normal, reference);
            Surface {
                at: Arc::new(move |u, v| add(origin, add(scale(x, u), scale(y, v)))),
                domain: infinite,
                spans: [1, 1],
                invert: Some(Arc::new(move |p| {
                    let d = sub(p, origin);
                    [dot(d, x), dot(d, y)]
                })),
                period: [None, None],
                native: false,
            }
        };
        let surface = match e.kind {
            128 => {
                let (k1, k2) = (Reader::int(e, 0).max(0) as usize, Reader::int(e, 1).max(0) as usize);
                let (d1, d2) = (Reader::int(e, 2).max(0) as usize, Reader::int(e, 3).max(0) as usize);
                let (c1, c2) = (k1 + 1, k2 + 1);
                let mut at = 9;
                let ku: Vec<f64> = (0..c1 + d1 + 1).map(|i| n(at + i)).collect();
                at += ku.len();
                let kv: Vec<f64> = (0..c2 + d2 + 1).map(|i| n(at + i)).collect();
                at += kv.len();
                let weights: Vec<f64> = (0..c1 * c2).map(|i| n(at + i)).map(|w| if w > 0.0 { w } else { 1.0 }).collect();
                at += weights.len();
                let points: Vec<[f64; 4]> = (0..c1 * c2)
                    .map(|i| {
                        let p = Reader::triple(e, at + 3 * i);
                        [p[0] * weights[i], p[1] * weights[i], p[2] * weights[i], weights[i]]
                    })
                    .collect();
                at += 3 * c1 * c2;
                let domain = [n(at), n(at + 1), n(at + 2), n(at + 3)];
                if d1 == 0 || d2 == 0 || !(domain[1] > domain[0]) || !(domain[3] > domain[2]) {
                    self.skip(128);
                    return None;
                }
                nurbs_surface(ku, kv, (d1, d2), (c1, c2), points, domain)
            }
            108 => {
                let normal = normalized([n(0), n(1), n(2)])?;
                let d = n(3) / length([n(0), n(1), n(2)]);
                plane(scale(normal, d), normal, None)
            }
            190 => {
                let origin = self.point(Reader::pointer(e, 0))?;
                let normal = self.direction(Reader::pointer(e, 1))?;
                plane(origin, normal, self.direction(Reader::pointer(e, 2)))
            }
            118 => {
                let first = path(&self.curve(Reader::pointer(e, 0), depth + 1)?);
                let mut second_pieces = self.curve(Reader::pointer(e, 1), depth + 1)?;
                if Reader::int(e, 2) == 1 {
                    second_pieces = reversed(second_pieces);
                }
                let segments = second_pieces.len().max(1) * 4;
                let second = path(&second_pieces);
                Surface {
                    at: Arc::new(move |u, v| lerp(first(u), second(u), v)),
                    domain: [0.0, 1.0, 0.0, 1.0],
                    spans: [segments, 1],
                    invert: None,
                    period: [None, None],
                    native: false,
                }
            }
            120 => {
                let axis = self.curve(Reader::pointer(e, 0), depth + 1)?;
                let (a, b) = (axis.first()?.start(), axis.last()?.end());
                let direction = normalized(sub(b, a))?;
                let generatrix = self.curve(Reader::pointer(e, 1), depth + 1)?;
                let segments = generatrix.len().max(1) * 4;
                let along = path(&generatrix);
                let (start, end) = (n(2), n(3));
                let whole = (end - start - TAU).abs() < 1e-9;
                Surface {
                    at: Arc::new(move |u, angle| {
                        // Rodrigues: the generatrix point turned about the axis.
                        let p = sub(along(u), a);
                        let (s, c) = angle.sin_cos();
                        let turned = add(add(scale(p, c), scale(cross(direction, p), s)), scale(direction, dot(direction, p) * (1.0 - c)));
                        add(a, turned)
                    }),
                    domain: [0.0, 1.0, start, end],
                    spans: [segments, ((end - start).abs() / (PI / 8.0)).ceil().max(1.0) as usize],
                    invert: None,
                    period: [None, whole.then_some(TAU)],
                    native: false,
                }
            }
            122 => {
                let directrix = self.curve(Reader::pointer(e, 0), depth + 1)?;
                let segments = directrix.len().max(1) * 4;
                let along = path(&directrix);
                let lift = sub(Reader::triple(e, 1), directrix.first()?.start());
                Surface {
                    at: Arc::new(move |u, v| add(along(u), scale(lift, v))),
                    domain: [0.0, 1.0, 0.0, 1.0],
                    spans: [segments, 1],
                    invert: None,
                    period: [None, None],
                    native: false,
                }
            }
            192 | 194 => {
                let origin = self.point(Reader::pointer(e, 0))?;
                let axis = self.direction(Reader::pointer(e, 1))?;
                let radius = n(2);
                // The cone's half-angle, in degrees.
                let (slope, reference) = if e.kind == 194 { ((n(3).to_radians()).tan(), Reader::pointer(e, 4)) } else { (0.0, Reader::pointer(e, 3)) };
                let (x, y) = frame(axis, self.direction(reference));
                let apex = if slope > 0.0 { -radius / slope } else { f64::NEG_INFINITY };
                Surface {
                    at: Arc::new(move |angle, h| {
                        let r = radius + h * slope;
                        add(origin, add(add(scale(x, r * angle.cos()), scale(y, r * angle.sin())), scale(axis, h)))
                    }),
                    domain: [f64::NEG_INFINITY, f64::INFINITY, apex, f64::INFINITY],
                    spans: [16, 1],
                    invert: Some(Arc::new(move |p| {
                        let d = sub(p, origin);
                        [dot(d, y).atan2(dot(d, x)), dot(d, axis)]
                    })),
                    period: [Some(TAU), None],
                    native: false,
                }
            }
            196 => {
                let centre = self.point(Reader::pointer(e, 0))?;
                let radius = n(1);
                let axis = self.direction(Reader::pointer(e, 2)).unwrap_or([0.0, 0.0, 1.0]);
                let (x, y) = frame(axis, self.direction(Reader::pointer(e, 3)));
                Surface {
                    at: Arc::new(move |lon, lat| {
                        let ring = add(scale(x, lon.cos()), scale(y, lon.sin()));
                        add(centre, scale(add(scale(ring, lat.cos()), scale(axis, lat.sin())), radius))
                    }),
                    domain: [f64::NEG_INFINITY, f64::INFINITY, -PI / 2.0, PI / 2.0],
                    spans: [16, 8],
                    invert: Some(Arc::new(move |p| {
                        let d = sub(p, centre);
                        let (a, b, c) = (dot(d, x), dot(d, y), dot(d, axis));
                        [b.atan2(a), c.atan2((a * a + b * b).sqrt())]
                    })),
                    period: [Some(TAU), None],
                    native: false,
                }
            }
            198 => {
                let centre = self.point(Reader::pointer(e, 0))?;
                let axis = self.direction(Reader::pointer(e, 1))?;
                let (major, minor) = (n(2), n(3));
                let (x, y) = frame(axis, self.direction(Reader::pointer(e, 4)));
                Surface {
                    at: Arc::new(move |u, v| {
                        let ring = add(scale(x, u.cos()), scale(y, u.sin()));
                        add(centre, add(scale(ring, major + minor * v.cos()), scale(axis, minor * v.sin())))
                    }),
                    domain: [f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY],
                    spans: [16, 16],
                    invert: Some(Arc::new(move |p| {
                        let d = sub(p, centre);
                        let (a, b, c) = (dot(d, x), dot(d, y), dot(d, axis));
                        [b.atan2(a), c.atan2((a * a + b * b).sqrt() - major)]
                    })),
                    period: [Some(TAU), Some(TAU)],
                    native: false,
                }
            }
            other => {
                self.skip(other);
                return None;
            }
        };
        Some(surface.mapped(&self.placement(e)))
    }

    /// A curve on a surface (142) as a trimming segment: its form in the
    /// surface's parameters where the surface defines them, else its form
    /// in space.
    fn on_surface(&self, de: u64, surface: &Surface) -> Option<Segment> {
        let e = self.get(de)?;
        if e.kind != 142 {
            return self.curve(de, 0).map(Segment::Model);
        }
        let (b, c) = (Reader::pointer(e, 2), Reader::pointer(e, 3));
        let placement = self.placement(e);
        let in_space = || self.curve(c, 0).map(|p| Segment::Model(p.into_iter().map(|q| q.mapped(&placement)).collect()));
        if surface.native && b != 0 {
            if let Some(pieces) = self.curve(b, 0) {
                return Some(Segment::Params(pieces));
            }
        }
        if c != 0 { in_space() } else { None }
    }

    /// A colour: one of IGES's eight by number, or a definition (314).
    fn colour(&self, e: &Entity) -> Option<[f32; 3]> {
        Some(match e.colour {
            1 => [0.0, 0.0, 0.0],
            2 => [1.0, 0.0, 0.0],
            3 => [0.0, 1.0, 0.0],
            4 => [0.0, 0.0, 1.0],
            5 => [1.0, 1.0, 0.0],
            6 => [1.0, 0.0, 1.0],
            7 => [0.0, 1.0, 1.0],
            8 => [1.0, 1.0, 1.0],
            c if c < 0 => {
                let d = self.get(c.unsigned_abs()).filter(|d| d.kind == 314)?;
                [0, 1, 2].map(|i| (Reader::number(d, i) / 100.0).clamp(0.0, 1.0) as f32)
            }
            _ => return None,
        })
    }

    /// An entity's name: a name property (406 form 15) among the pointers
    /// after its `used` parameters, or its label.
    fn name(&self, e: &Entity, used: usize) -> Option<String> {
        let associations = Reader::int(e, used).max(0) as usize;
        let properties_at = used + 1 + associations;
        let properties = Reader::int(e, properties_at).max(0) as usize;
        for i in 0..properties {
            if let Some(p) = self.get(Reader::pointer(e, properties_at + 1 + i)) {
                if p.kind == 406 && p.form == 15 {
                    if let Some(name) = Reader::text(p, 1) {
                        return Some(name);
                    }
                }
            }
        }
        Some(e.label.trim().to_string()).filter(|l| !l.is_empty())
    }

    /// The faces an entity draws: a trimmed surface, a surface standing
    /// alone, a bounded plane, a solid or a shell.
    fn faces(&self, de: u64, out: &mut Vec<Face>) {
        let Some(e) = self.get(de) else { return };
        let placement = self.placement(e);
        let start = out.len();
        match e.kind {
            144 => {
                let Some(surface) = self.surface(Reader::pointer(e, 0), 0) else { return };
                let (outer_given, inner) = (Reader::int(e, 1) != 0, Reader::int(e, 2).max(0) as usize);
                let mut loops = Vec::new();
                if outer_given {
                    match self.on_surface(Reader::pointer(e, 3), &surface) {
                        Some(segment) => loops.push(vec![segment]),
                        None => return,
                    }
                } else if !surface.bounded() {
                    return;
                } else {
                    // The domain's own boundary, given as a loop so that
                    // holes cut it like any other.
                    let [u0, u1, v0, v1] = surface.domain;
                    let side = |a: P2, b: P2| Piece::line([a[0], a[1], 0.0], [b[0], b[1], 0.0]);
                    loops.push(vec![Segment::Params(vec![side([u0, v0], [u1, v0]), side([u1, v0], [u1, v1]), side([u1, v1], [u0, v1]), side([u0, v1], [u0, v0])])]);
                }
                for i in 0..inner {
                    if let Some(segment) = self.on_surface(Reader::pointer(e, 4 + i), &surface) {
                        loops.push(vec![segment]);
                    }
                }
                let surface_entity = self.get(Reader::pointer(e, 0));
                out.push(Face {
                    surface,
                    loops,
                    reversed: false,
                    colour: self.colour(e).or_else(|| surface_entity.and_then(|s| self.colour(s))),
                    body: None,
                    name: self.name(e, 4 + inner),
                });
            }
            128 | 118 | 120 | 122 => {
                let Some(surface) = self.surface(de, 0) else { return };
                if surface.bounded() {
                    out.push(Face { surface, loops: Vec::new(), reversed: false, colour: self.colour(e), body: None, name: Some(e.label.clone()).filter(|l| !l.is_empty()) });
                }
            }
            108 if e.form == 1 => {
                let Some(surface) = self.surface(de, 0) else { return };
                let Some(boundary) = self.curve(Reader::pointer(e, 4), 0) else { return };
                // The plane's own matrix placed the surface; its boundary
                // is in the same space before it.
                let boundary = boundary.into_iter().map(|p| p.mapped(&self.placement(e))).collect();
                out.push(Face { surface, loops: vec![vec![Segment::Model(boundary)]], reversed: false, colour: self.colour(e), body: None, name: None });
            }
            186 => {
                let reversed = Reader::int(e, 1) == 0;
                let voids = Reader::int(e, 2).max(0) as usize;
                let colour = self.colour(e);
                let name = self.name(e, 3 + 2 * voids);
                self.shell(Reader::pointer(e, 0), reversed, de, out);
                for i in 0..voids {
                    self.shell(Reader::pointer(e, 3 + 2 * i), Reader::int(e, 4 + 2 * i) == 0, de, out);
                }
                for face in &mut out[start..] {
                    face.colour = face.colour.or(colour);
                    face.name = name.clone();
                }
            }
            514 => self.shell(de, false, de, out),
            // An unbounded plane draws nothing.
            108 => {}
            other => self.skip(other),
        }
        for face in &mut out[start..] {
            if placement != Affine::IDENTITY && matches!(e.kind, 144 | 186 | 514) {
                face.surface = face.surface.clone().mapped(&placement);
                for segment in face.loops.iter_mut().flatten() {
                    if let Segment::Model(pieces) = segment {
                        *pieces = pieces.drain(..).map(|p| p.mapped(&placement)).collect();
                    }
                }
            }
        }
    }

    /// A shell's faces (514 of 510s), each with its loops (508) of edges
    /// (504).
    fn shell(&self, de: u64, flipped: bool, body: u64, out: &mut Vec<Face>) {
        let Some(shell) = self.get(de).filter(|e| e.kind == 514) else { return };
        let count = Reader::int(shell, 0).max(0) as usize;
        for i in 0..count {
            let Some(face) = self.get(Reader::pointer(shell, 1 + 2 * i)).filter(|f| f.kind == 510) else { continue };
            let agrees = Reader::int(shell, 2 + 2 * i) != 0;
            let Some(surface) = self.surface(Reader::pointer(face, 0), 0) else { continue };
            let loop_count = Reader::int(face, 1).max(0) as usize;
            let mut loops = Vec::new();
            for l in 0..loop_count {
                let Some(lp) = self.get(Reader::pointer(face, 3 + l)).filter(|e| e.kind == 508) else { continue };
                let edges = Reader::int(lp, 0).max(0) as usize;
                let mut segments = Vec::new();
                let mut at = 1;
                for _ in 0..edges {
                    let (kind, list, index, forward) = (Reader::int(lp, at), Reader::pointer(lp, at + 1), Reader::int(lp, at + 2), Reader::int(lp, at + 3) != 0);
                    let curves = Reader::int(lp, at + 4).max(0) as usize;
                    let parameter_curve = (curves > 0).then(|| Reader::pointer(lp, at + 6));
                    at += 5 + 2 * curves;
                    if kind != 0 {
                        // A vertex standing for a loop (a cone's apex).
                        continue;
                    }
                    let Some(edge_list) = self.get(list).filter(|e| e.kind == 504) else { continue };
                    let index = (index.max(1) - 1) as usize;
                    let model = self.curve(Reader::pointer(edge_list, 1 + 5 * index), 0);
                    let orient = |pieces: Vec<Piece>| if forward { pieces } else { reversed(pieces) };
                    let given = parameter_curve.filter(|_| surface.native).and_then(|c| self.curve(c, 0));
                    match (given, model) {
                        (Some(pieces), _) => segments.push(Segment::Params(orient(pieces))),
                        (None, Some(pieces)) => segments.push(Segment::Model(orient(pieces))),
                        _ => {}
                    }
                }
                if !segments.is_empty() {
                    loops.push(segments);
                }
            }
            out.push(Face {
                surface,
                loops,
                reversed: flipped != !agrees,
                colour: self.colour(face),
                body: Some(body),
                name: None,
            });
        }
    }
}

/// Faces gathered into bodies: a solid's are one body; the rest, by the
/// corners they share, colour by colour.
fn bodies(faces: &[Face], tolerance: f64) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let join = |parent: &mut Vec<usize>, a: usize, b: usize| {
        let (ra, rb) = (root(parent, a), root(parent, b));
        if ra != rb {
            parent[ra.max(rb)] = ra.min(rb);
        }
    };
    let mut solids: BTreeMap<u64, usize> = BTreeMap::new();
    let mut cells: HashMap<[i64; 3], Vec<(usize, P3)>> = HashMap::new();
    let cell = |p: P3| p.map(|c| (c / tolerance).floor() as i64);
    for (i, face) in faces.iter().enumerate() {
        if let Some(body) = face.body {
            let first = *solids.entry(body).or_insert(i);
            join(&mut parent, first, i);
            continue;
        }
        for corner in face.corners() {
            let c = cell(corner);
            let mut matches = Vec::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        for &(j, q) in cells.get(&[c[0] + dx, c[1] + dy, c[2] + dz]).into_iter().flatten() {
                            if j != i && faces[j].colour == face.colour && length(sub(q, corner)) <= tolerance {
                                matches.push(j);
                            }
                        }
                    }
                }
            }
            for j in matches {
                join(&mut parent, i, j);
            }
            cells.entry(c).or_default().push((i, corner));
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..faces.len() {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    groups.into_values().collect()
}

/// Read an IGES file at the default quality.
pub fn read(bytes: &[u8], name: &str) -> Result<RawScene, ImportError> {
    read_with(bytes, name, Tessellation::default())
}

/// Read an IGES file, dividing its surfaces at `quality`.
pub fn read_with(bytes: &[u8], name: &str, quality: Tessellation) -> Result<RawScene, ImportError> {
    let file = parse(bytes).map_err(|e| ImportError::Malformed(format!("this is not an IGES file Tequila can read: {e}")))?;
    let reader = Reader { entities: file.entities, skipped: RefCell::new(BTreeMap::new()) };

    // What the file draws at the top: entities nothing else uses, not
    // hidden; and the subfigures placed.
    const DRAWN: [u32; 8] = [144, 128, 118, 120, 122, 108, 186, 514];
    let top: Vec<u64> = reader
        .entities
        .iter()
        .filter(|(_, e)| !e.dependent && !e.blanked && (DRAWN.contains(&e.kind) || e.kind == 408))
        .map(|(de, _)| *de)
        .collect();

    // A container's faces: the top level's, or a subfigure's.
    let collect = |members: &[u64]| -> Vec<Face> {
        let mut faces = Vec::new();
        for &de in members {
            if reader.get(de).is_some_and(|e| DRAWN.contains(&e.kind)) {
                reader.faces(de, &mut faces);
            }
        }
        faces
    };
    let top_faces = collect(&top);
    let mut definitions: BTreeMap<u64, Vec<Face>> = BTreeMap::new();
    for e in reader.entities.values().filter(|e| e.kind == 408) {
        let definition = Reader::pointer(e, 0);
        if let Some(d) = reader.get(definition).filter(|d| d.kind == 308) {
            if !definitions.contains_key(&definition) {
                let count = Reader::int(d, 2).max(0) as usize;
                let members: Vec<u64> = (0..count).map(|i| Reader::pointer(d, 3 + i)).collect();
                definitions.insert(definition, collect(&members));
            }
        }
    }

    let everything: Vec<P3> = top_faces.iter().chain(definitions.values().flatten()).flat_map(Face::corners).collect();
    if everything.is_empty() {
        return Err(ImportError::Unsupported("the IGES file has no surfaces or solids to mesh - only curves, drawings or annotation".into()));
    }
    let size = |points: &[P3]| {
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for p in points {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        length(sub(hi, lo))
    };
    let model_size = size(&everything).max(1e-9);

    let mut scene = RawScene { source_unit: file.unit, ..RawScene::default() };
    let mut materials: Vec<([f32; 3], usize)> = Vec::new();
    let mut warnings = Vec::new();
    let mut failed = 0usize;
    // A container's bodies as meshes, named.
    let mut mesh_container = |faces: &[Face], scene: &mut RawScene, counter: &mut usize| -> Vec<usize> {
        use rayon::prelude::*;
        let groups = bodies(faces, model_size * 1e-5);
        let tolerances: Vec<f64> = groups
            .iter()
            .map(|g| {
                let corners: Vec<P3> = g.iter().flat_map(|&i| faces[i].corners()).collect();
                (size(&corners) * quality.chord()).max(model_size * quality.chord() * 0.1)
            })
            .collect();
        let jobs: Vec<(usize, usize)> = groups.iter().enumerate().flat_map(|(g, members)| members.iter().map(move |&f| (g, f))).collect();
        let meshed: Vec<(usize, Meshed)> = jobs.par_iter().map(|&(g, f)| (g, tessellate(&faces[f], tolerances[g]))).collect();
        let mut out = Vec::new();
        for (g, members) in groups.iter().enumerate() {
            let mut mesh = RawMesh::default();
            for (_, m) in meshed.iter().filter(|(owner, _)| *owner == g) {
                if m.triangles.is_empty() {
                    failed += 1;
                    continue;
                }
                let base = mesh.positions.len() as u32;
                let to = |p: P3| vec3((p[0] * file.scale) as f32, (p[1] * file.scale) as f32, (p[2] * file.scale) as f32);
                mesh.positions.extend(m.positions.iter().map(|p| to(*p)));
                mesh.normals.extend(m.normals.iter().map(|n| vec3(n[0] as f32, n[1] as f32, n[2] as f32)));
                mesh.indices.extend(m.triangles.iter().flat_map(|t| t.map(|i| i + base)));
                mesh.edge_lines.extend(m.edges.iter().map(|p| to(*p)));
            }
            if mesh.indices.is_empty() {
                continue;
            }
            *counter += 1;
            let first = &faces[members[0]];
            mesh.name = first.name.clone().unwrap_or_else(|| format!("Body {counter}"));
            // A colour as a plastic, made once; no colour, the default
            // material, as for STEP.
            mesh.material = first.colour.map(|c| match materials.iter().find(|(seen, _)| *seen == c) {
                Some((_, index)) => *index,
                None => {
                    let label = format!("#{:02X}{:02X}{:02X}", (c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8);
                    let base = Spectrum::from_srgb_encoded(vec3(c[0], c[1], c[2]));
                    scene.materials.push(MaterialDefinition::new(label, MaterialType::Plastic, Material::plastic(base, 0.2)));
                    materials.push((c, scene.materials.len() - 1));
                    scene.materials.len() - 1
                }
            });
            scene.meshes.push(mesh);
            out.push(scene.meshes.len() - 1);
        }
        out
    };

    let mut counter = 0;
    scene.nodes.push(RawNode::group(name));
    scene.roots.push(0);
    for mesh in mesh_container(&top_faces, &mut scene, &mut counter) {
        let mut node = RawNode::part(scene.meshes[mesh].name.clone(), mesh);
        node.parent = Some(0);
        scene.nodes.push(node);
    }
    // Subfigures: each definition meshed once, placed where each instance
    // puts it.
    let mut placed_meshes: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (definition, faces) in &definitions {
        placed_meshes.insert(*definition, mesh_container(faces, &mut scene, &mut counter));
    }
    for de in &top {
        let Some(e) = reader.get(*de).filter(|e| e.kind == 408) else { continue };
        let definition = Reader::pointer(e, 0);
        let Some(d) = reader.get(definition) else { continue };
        let s = Reader::number(e, 4);
        let s = if s == 0.0 { 1.0 } else { s };
        let local = Affine { r: [[s, 0.0, 0.0], [0.0, s, 0.0], [0.0, 0.0, s]], t: Reader::triple(e, 1) };
        let mut group = RawNode::group(Reader::text(d, 1).or_else(|| reader.name(e, 5)).unwrap_or_else(|| "Subfigure".into()));
        group.parent = Some(0);
        group.transform = local.then(&reader.placement(e)).to_transform(file.scale);
        scene.nodes.push(group);
        let parent = scene.nodes.len() - 1;
        for &mesh in placed_meshes.get(&definition).into_iter().flatten() {
            let mut node = RawNode::part(scene.meshes[mesh].name.clone(), mesh);
            node.parent = Some(parent);
            scene.nodes.push(node);
        }
    }

    if failed > 0 {
        warnings.push(format!("{failed} faces could not be divided and are missing"));
    }
    let skipped = reader.skipped.borrow();
    if !skipped.is_empty() {
        let list: Vec<String> = skipped.iter().map(|(k, n)| format!("type {k} ×{n}")).collect();
        warnings.push(format!("entities Tequila does not read were left out: {}", list.join(", ")));
    }
    if scene.meshes.is_empty() {
        return Err(ImportError::Malformed(format!(
            "no face in the IGES file could be divided{}",
            warnings.first().map(|w| format!(" ({w})")).unwrap_or_default()
        )));
    }
    scene.warnings = warnings;
    scene.tessellation = Some(quality.label().to_string());
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IGES files written for the tests: entities in order, each given the
    /// directory number it will have (1, 3, 5, ...).
    #[derive(Default)]
    struct Writer {
        entities: Vec<(u32, i64, String, u64, i64, bool, String)>,
        unit: i64,
    }

    impl Writer {
        fn new(unit: i64) -> Writer {
            Writer { unit, ..Writer::default() }
        }

        /// A dependent entity: used by another, not drawn alone.
        fn part(&mut self, kind: u32, form: i64, params: &str) -> u64 {
            self.full(kind, form, params, 0, 0, true, "")
        }

        /// An independent entity: drawn.
        fn top(&mut self, kind: u32, form: i64, params: &str) -> u64 {
            self.full(kind, form, params, 0, 0, false, "")
        }

        #[allow(clippy::too_many_arguments)]
        fn full(&mut self, kind: u32, form: i64, params: &str, matrix: u64, colour: i64, dependent: bool, label: &str) -> u64 {
            self.entities.push((kind, form, params.to_string(), matrix, colour, dependent, label.to_string()));
            2 * self.entities.len() as u64 - 1
        }

        fn text(&self) -> String {
            let mut out = String::new();
            let line = |out: &mut String, data: &str, letter: char, n: usize| out.push_str(&format!("{data:<72}{letter}{n:>7}\n"));
            line(&mut out, "Tequila test file", 'S', 1);
            let global = format!(
                "1H,,1H;,4Htest,8Htest.igs,8HTequila,3H1.0,32,38,6,308,15,4Htest,1.,{},2HMM,1,0.1,15H20261011.120000,1.E-06,1000.,4Htest,4Htest,11,0,15H20261011.120000;",
                self.unit
            );
            let g: Vec<String> = global.as_bytes().chunks(72).map(|c| String::from_utf8_lossy(c).into_owned()).collect();
            for (i, chunk) in g.iter().enumerate() {
                line(&mut out, chunk, 'G', i + 1);
            }
            // Parameter lines first, to know where each entity's start.
            let mut p_lines: Vec<String> = Vec::new();
            let mut starts: Vec<(usize, usize)> = Vec::new();
            for (i, (kind, _, params, ..)) in self.entities.iter().enumerate() {
                let record = if params.is_empty() { format!("{kind};") } else { format!("{kind},{params};") };
                let de = 2 * i + 1;
                let start = p_lines.len() + 1;
                let mut current = String::new();
                for token in record.split_inclusive(',') {
                    if current.len() + token.len() > 64 && !current.is_empty() {
                        p_lines.push(format!("{current:<64} {de:>7}"));
                        current.clear();
                    }
                    // A token longer than a line (a long string) is split.
                    let mut rest = token;
                    while rest.len() > 64 {
                        p_lines.push(format!("{:<64} {de:>7}", &rest[..64]));
                        rest = &rest[64..];
                    }
                    current.push_str(rest);
                }
                p_lines.push(format!("{current:<64} {de:>7}"));
                starts.push((start, p_lines.len() + 1 - start));
            }
            for (i, (kind, form, _, matrix, colour, dependent, label)) in self.entities.iter().enumerate() {
                let (start, count) = starts[i];
                let status = if *dependent { "00010000" } else { "00000000" };
                line(&mut out, &format!("{kind:>8}{start:>8}{:>8}{:>8}{:>8}{:>8}{matrix:>8}{:>8}{status:>8}", 0, 0, 0, 0, 0), 'D', 2 * i + 1);
                line(&mut out, &format!("{kind:>8}{:>8}{colour:>8}{count:>8}{form:>8}{:>8}{:>8}{label:>8}{:>8}", 0, "", "", 0), 'D', 2 * i + 2);
            }
            for (i, l) in p_lines.iter().enumerate() {
                line(&mut out, l, 'P', i + 1);
            }
            let d = 2 * self.entities.len();
            line(&mut out, &format!("S{:>7}G{:>7}D{d:>7}P{:>7}", 1, g.len(), p_lines.len()), 'T', 1);
            out
        }
    }

    fn area(mesh: &RawMesh) -> f64 {
        mesh.indices
            .chunks(3)
            .map(|t| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize]);
                ((b - a).cross(c - a).length() * 0.5) as f64
            })
            .sum()
    }

    fn volume(mesh: &RawMesh) -> f64 {
        mesh.indices
            .chunks(3)
            .map(|t| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize]);
                (a.dot(b.cross(c)) / 6.0) as f64
            })
            .sum()
    }

    fn to_mesh(meshed: &Meshed) -> RawMesh {
        RawMesh {
            positions: meshed.positions.iter().map(|p| vec3(p[0] as f32, p[1] as f32, p[2] as f32)).collect(),
            indices: meshed.triangles.iter().flatten().copied().collect(),
            ..RawMesh::default()
        }
    }

    const SQUARE: &str = "1,1,1,1,0,0,1,0,0,0.,0.,1.,1.,0.,0.,1.,1.,1.,1.,1.,1.,0.,0.,0.,10.,0.,0.,0.,10.,0.,10.,10.,0.,0.,1.,0.,1.";

    #[test]
    fn a_trimmed_patch_has_its_hole() {
        let mut w = Writer::new(2);
        let surface = w.part(128, 0, SQUARE);
        // A circle in the patch's parameters: centre (0.5, 0.5), radius 0.2.
        let circle = w.part(100, 0, "0.,0.5,0.5,0.7,0.5,0.7,0.5");
        let hole = w.part(142, 0, &format!("1,{surface},{circle},0,1"));
        let colour = w.part(314, 0, "80.,20.,10.,4HRust");
        w.full(144, 0, &format!("{surface},0,1,0,{hole}"), 0, -(colour as i64), false, "PANEL");
        let scene = read(w.text().as_bytes(), "patch").expect("reads");
        scene.validate().unwrap();
        assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);
        assert_eq!(scene.source_unit, Some(LengthUnit::Millimetre));
        assert_eq!(scene.meshes.len(), 1);
        let mesh = &scene.meshes[0];
        let expected = 100.0 - PI * 2.0 * 2.0;
        assert!((area(mesh) - expected).abs() < expected * 2e-3, "area {} not {expected}", area(mesh));
        assert_eq!(mesh.name, "PANEL");
        let material = &scene.materials[mesh.material.unwrap()];
        let rgb = material.material().base_color.to_linear_srgb();
        assert!(rgb.x > rgb.y * 3.0 && rgb.y > rgb.z, "{rgb:?}");
        // Its edges: the square's four sides and the circle.
        let edge_length: f64 = mesh.edge_lines.chunks(2).map(|s| (s[1] - s[0]).length() as f64).sum();
        let expected = 40.0 + TAU * 2.0;
        assert!((edge_length - expected).abs() < expected * 2e-3, "edges {edge_length} not {expected}");
    }

    #[test]
    fn a_rational_patch_is_an_exact_quarter_cylinder() {
        let mut w = Writer::new(2);
        let h = std::f64::consts::FRAC_1_SQRT_2;
        w.top(
            128,
            0,
            &format!("2,1,2,1,0,0,0,0,0,0.,0.,0.,1.,1.,1.,0.,0.,1.,1.,1.,{h},1.,1.,{h},1.,10.,0.,0.,10.,10.,0.,0.,10.,0.,10.,0.,10.,10.,10.,10.,0.,10.,10.,0.,1.,0.,1."),
        );
        let scene = read(w.text().as_bytes(), "quarter").expect("reads");
        let mesh = &scene.meshes[0];
        for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
            let r = (p.x * p.x + p.y * p.y).sqrt();
            assert!((r - 10.0).abs() < 1e-4, "off the cylinder: {p:?}");
            let radial = vec3(p.x / r, p.y / r, 0.0);
            assert!(n.dot(radial) > 0.999, "normal {n:?} at {p:?}");
        }
        let expected = PI / 2.0 * 10.0 * 10.0;
        assert!((area(mesh) - expected).abs() < expected * 2e-3, "area {}", area(mesh));
        // Divided finely enough to follow the curve: Medium is a
        // thousandth of the size.
        let size = 10.0 * 3f64.sqrt();
        let worst = mesh
            .indices
            .chunks(3)
            .map(|t| {
                let c = (mesh.positions[t[0] as usize] + mesh.positions[t[1] as usize] + mesh.positions[t[2] as usize]) / 3.0;
                10.0 - (c.x * c.x + c.y * c.y).sqrt() as f64
            })
            .fold(0.0, f64::max);
        assert!(worst < size * 1e-3 * 1.5, "facets stray {worst}");
    }

    #[test]
    fn a_surface_curved_both_ways_is_divided_inside_too() {
        // A quarter circle turned into the XZ plane by a matrix, revolved
        // about Z: a hemisphere of radius 5.
        let mut w = Writer::new(2);
        let axis = w.part(110, 0, "0.,0.,0.,0.,0.,10.");
        let swap = w.part(124, 0, "1.,0.,0.,0.,0.,0.,1.,0.,0.,1.,0.,0.");
        let arc = w.full(100, 0, "0.,0.,0.,5.,0.,0.,5.", swap, 0, true, "");
        w.top(120, 0, &format!("{axis},{arc},0.,{TAU}"));
        let scene = read(w.text().as_bytes(), "dome").expect("reads");
        let mesh = &scene.meshes[0];
        for p in &mesh.positions {
            assert!((p.length() - 5.0).abs() < 1e-4, "off the sphere: {p:?}");
        }
        // Facets inside a sphere fall short of its area by about twice
        // the chord over the radius; Medium's chord is a thousandth of the
        // part's size, here about 10.
        let expected = TAU * 25.0;
        assert!((area(mesh) - expected).abs() < expected * 6e-3, "area {} not {expected}", area(mesh));
        // Every facet close to the sphere, not only its corners.
        let worst = mesh
            .indices
            .chunks(3)
            .map(|t| 5.0 - ((mesh.positions[t[0] as usize] + mesh.positions[t[1] as usize] + mesh.positions[t[2] as usize]) / 3.0).length() as f64)
            .fold(0.0, f64::max);
        assert!(worst < 10.0 * 1e-3 * 1.5, "facets stray {worst}");
    }

    #[test]
    fn a_patch_trimmed_in_space_is_found_on_its_surface() {
        // A triangle given only as lines in space, on the flat patch, its
        // corners off any grid a search might start from.
        let mut w = Writer::new(2);
        let surface = w.part(128, 0, SQUARE);
        let a = w.part(110, 0, "1.3,0.7,0.,9.1,2.2,0.");
        let b = w.part(110, 0, "9.1,2.2,0.,3.7,8.9,0.");
        let c = w.part(110, 0, "3.7,8.9,0.,1.3,0.7,0.");
        let composite = w.part(102, 0, &format!("3,{a},{b},{c}"));
        let outer = w.part(142, 0, &format!("1,{surface},0,{composite},2"));
        w.top(144, 0, &format!("{surface},1,0,{outer}"));
        let scene = read(w.text().as_bytes(), "triangle").expect("reads");
        let expected = 0.5 * ((9.1 - 1.3) * (8.9 - 0.7) - (3.7 - 1.3) * (2.2 - 0.7));
        assert!((area(&scene.meshes[0]) - expected).abs() < 1e-4, "{} not {expected}", area(&scene.meshes[0]));
    }

    #[test]
    fn a_surface_of_revolution_closes_round_its_seam() {
        let mut w = Writer::new(2);
        let axis = w.part(110, 0, "0.,0.,0.,0.,0.,10.");
        let line = w.part(110, 0, "5.,0.,0.,5.,0.,10.");
        w.top(120, 0, &format!("{axis},{line},0.,{TAU}"));
        let scene = read(w.text().as_bytes(), "tube").expect("reads");
        let mesh = &scene.meshes[0];
        let expected = TAU * 5.0 * 10.0;
        assert!((area(mesh) - expected).abs() < expected * 3e-3, "area {}", area(mesh));
        // Its two circles are edges; the seam is not.
        let edge_length: f64 = mesh.edge_lines.chunks(2).map(|s| (s[1] - s[0]).length() as f64).sum();
        assert!((edge_length - 2.0 * TAU * 5.0).abs() < 0.5, "edges {edge_length}");
    }

    /// A 10 mm cube as a solid: planes, lines, an edge list, loops, faces,
    /// a shell. The bottom face's plane faces in, and the shell says so.
    /// Off the origin, so that every face counts in its volume.
    fn cube() -> String {
        let mut w = Writer::new(2);
        let corners: [P3; 8] = [[0., 0., 0.], [10., 0., 0.], [10., 10., 0.], [0., 10., 0.], [0., 0., 10.], [10., 0., 10.], [10., 10., 10.], [0., 10., 10.]]
            .map(|p| add(p, [3.0, 4.0, 5.0]));
        // Each face counter-clockwise seen from outside, with its outward normal.
        let faces: [([usize; 4], P3); 6] = [
            ([0, 3, 2, 1], [0., 0., -1.]),
            ([4, 5, 6, 7], [0., 0., 1.]),
            ([0, 1, 5, 4], [0., -1., 0.]),
            ([2, 3, 7, 6], [0., 1., 0.]),
            ([1, 2, 6, 5], [1., 0., 0.]),
            ([3, 0, 4, 7], [-1., 0., 0.]),
        ];
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for (cycle, _) in &faces {
            for i in 0..4 {
                let (a, b) = (cycle[i], cycle[(i + 1) % 4]);
                if !edges.contains(&(a, b)) && !edges.contains(&(b, a)) {
                    edges.push((a, b));
                }
            }
        }
        let f = |p: P3| format!("{:.1},{:.1},{:.1}", p[0], p[1], p[2]);
        let vertex_list = w.part(502, 1, &format!("8,{}", corners.iter().map(|p| f(*p)).collect::<Vec<_>>().join(",")));
        let lines: Vec<u64> = edges.iter().map(|(a, b)| w.part(110, 0, &format!("{},{}", f(corners[*a]), f(corners[*b])))).collect();
        let tuples: Vec<String> = edges.iter().zip(&lines).map(|((a, b), l)| format!("{l},{vertex_list},{},{vertex_list},{}", a + 1, b + 1)).collect();
        let edge_list = w.part(504, 1, &format!("{},{}", edges.len(), tuples.join(",")));
        let mut shell_entries = Vec::new();
        for (i, (cycle, normal)) in faces.iter().enumerate() {
            let inward = i == 0;
            let point = w.part(116, 0, &f(corners[cycle[0]]));
            let direction = w.part(123, 0, &f(if inward { scale(*normal, -1.0) } else { *normal }));
            let plane = w.part(190, 0, &format!("{point},{direction}"));
            let tuples: Vec<String> = (0..4)
                .map(|k| {
                    let (a, b) = (cycle[k], cycle[(k + 1) % 4]);
                    let (index, forward) = match edges.iter().position(|e| *e == (a, b)) {
                        Some(i) => (i, 1),
                        None => (edges.iter().position(|e| *e == (b, a)).unwrap(), 0),
                    };
                    format!("0,{edge_list},{},{forward},0", index + 1)
                })
                .collect();
            let lp = w.part(508, 1, &format!("4,{}", tuples.join(",")));
            let face = w.part(510, 1, &format!("{plane},1,1,{lp}"));
            shell_entries.push(format!("{face},{}", if inward { 0 } else { 1 }));
        }
        let shell = w.part(514, 1, &format!("6,{}", shell_entries.join(",")));
        let name = w.part(406, 15, "1,9HBox, bolt");
        w.top(186, 0, &format!("{shell},1,0,0,1,{name}"));
        w.text()
    }

    #[test]
    fn a_solid_is_closed_and_faces_out() {
        let scene = read(cube().as_bytes(), "cube").expect("reads");
        assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);
        assert_eq!(scene.meshes.len(), 1, "one solid, one part");
        let mesh = &scene.meshes[0];
        // The name, though it holds the delimiter.
        assert_eq!(mesh.name, "Box, bolt");
        assert!((volume(mesh) - 1000.0).abs() < 1e-3, "volume {}", volume(mesh));
        assert!((area(mesh) - 600.0).abs() < 1e-3);
        for t in mesh.indices.chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize]);
            let face = (b - a).cross(c - a).normalize();
            assert!(face.dot(mesh.normals[t[0] as usize]) > 0.99, "a normal disagrees with its triangle");
        }
        // Twelve edges, each drawn by its two faces.
        let edge_length: f32 = mesh.edge_lines.chunks(2).map(|s| (s[1] - s[0]).length()).sum();
        assert!((edge_length - 240.0).abs() < 1e-3, "{edge_length}");
    }

    #[test]
    fn a_subfigure_is_meshed_once_and_placed_twice() {
        let mut w = Writer::new(1);
        let surface = w.part(128, 0, SQUARE);
        let definition = w.part(308, 0, &format!("0,5HPlate,1,{surface}"));
        w.top(408, 0, &format!("{definition},0.,0.,0.,1."));
        w.top(408, 0, &format!("{definition},20.,0.,0.,1."));
        let scene = read(w.text().as_bytes(), "plates").expect("reads");
        assert_eq!(scene.source_unit, Some(LengthUnit::Inch));
        assert_eq!(scene.meshes.len(), 1);
        let groups: Vec<usize> = scene.nodes.iter().enumerate().filter(|(_, n)| n.name == "Plate").map(|(i, _)| i).collect();
        assert_eq!(groups.len(), 2);
        let parts: Vec<&RawNode> = scene.nodes.iter().filter(|n| n.mesh == Some(0)).collect();
        assert_eq!(parts.len(), 2);
        let x = |i: usize| scene.nodes[i].transform.point(vec3(0.0, 0.0, 0.0)).x;
        assert!((x(groups[1]) - x(groups[0]) - 20.0).abs() < 1e-6);
    }

    #[test]
    fn numbers_in_fortran_form_and_matrices_are_read() {
        let mut w = Writer::new(2);
        // Lifted 5 along z by a matrix, written with D exponents.
        let matrix = w.part(124, 0, "1.0D0,0.,0.,0.,0.,1.D0,0.,0.,0.,0.,1.0D+00,5.0D0");
        w.full(128, 0, SQUARE, matrix, 0, false, "");
        let scene = read(w.text().as_bytes(), "lifted").expect("reads");
        assert!(scene.meshes[0].positions.iter().all(|p| (p.z - 5.0).abs() < 1e-6));
    }

    #[test]
    fn compressed_and_broken_files_are_refused_by_name() {
        let compressed = format!("{:<72}C{:>7}\n", "compressed", 1);
        let error = read(compressed.as_bytes(), "c").unwrap_err().to_string();
        assert!(error.contains("compressed"), "{error}");
        assert!(read("not an IGES file\n".as_bytes(), "x").is_err());
        // Only curves: nothing to mesh.
        let mut w = Writer::new(2);
        w.top(110, 0, "0.,0.,0.,1.,1.,1.");
        assert!(matches!(read(w.text().as_bytes(), "wire"), Err(ImportError::Unsupported(_))));
    }

    /// A cylinder of radius 5 along z, as an analytic surface.
    fn cylinder() -> Surface {
        let r = 5.0;
        Surface {
            at: Arc::new(move |a: f64, h: f64| [r * a.cos(), r * a.sin(), h]),
            domain: [f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY],
            spans: [16, 1],
            invert: Some(Arc::new(|p: P3| [p[1].atan2(p[0]), p[2]])),
            period: [Some(TAU), None],
            native: false,
        }
    }

    fn circle_at(z: f64, radius: f64, from: f64) -> Vec<Piece> {
        vec![Piece::new(move |t| [radius * t.cos(), radius * t.sin(), z], from, from + TAU, 16)]
    }

    #[test]
    fn two_circles_round_a_cylinder_bound_its_band() {
        // Each circle starting somewhere else, one each way.
        let face = Face {
            surface: cylinder(),
            loops: vec![vec![Segment::Model(circle_at(0.0, 5.0, 0.3))], vec![Segment::Model(reversed(circle_at(10.0, 5.0, 2.0)))]],
            reversed: false,
            colour: None,
            body: None,
            name: None,
        };
        let mesh = to_mesh(&tessellate(&face, 0.01));
        let expected = TAU * 5.0 * 10.0;
        assert!((area(&mesh) - expected).abs() < expected * 3e-3, "area {}", area(&mesh));
    }

    #[test]
    fn one_circle_round_a_cone_closes_at_its_apex() {
        // A cone narrowing from radius 5 at z 0 to its apex at z 5.
        let surface = Surface {
            at: Arc::new(|a: f64, h: f64| {
                let r = 5.0 - h;
                [r * a.cos(), r * a.sin(), h]
            }),
            domain: [f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY, 5.0],
            spans: [16, 1],
            invert: Some(Arc::new(|p: P3| [p[1].atan2(p[0]), p[2]])),
            period: [Some(TAU), None],
            native: false,
        };
        let face = Face { surface, loops: vec![vec![Segment::Model(circle_at(0.0, 5.0, 1.0))]], reversed: false, colour: None, body: None, name: None };
        let mesh = to_mesh(&tessellate(&face, 0.005));
        let expected = PI * 5.0 * 50f64.sqrt();
        assert!((area(&mesh) - expected).abs() < expected * 5e-3, "area {} not {expected}", area(&mesh));
    }
}
