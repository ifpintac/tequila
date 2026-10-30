//! KeyShot scenes (`.bip`, and `.ksp`: a ZIP of one with the pictures it
//! reads), as a model with its setup.
//!
//! What comes over:
//!
//! * the scene tree: its model set, the models in it, their layers and
//!   objects, with their names, placements and visibility. A KeyShot part
//!   is one face of a CAD object, so an object's faces are gathered into
//!   one Tequila part, a part per material where an object has several;
//! * the geometry: trimmed NURBS faces divided as IGES's are
//!   (`crate::iges::divide_nurbs`), at the import's quality, their boundary
//!   curves kept as the model's edges; and triangle meshes as they are;
//! * the materials, through the `.kmp` converter (`crate::keyshot`), and
//!   multi-materials as Tequila's switchable variants;
//! * the environments made in KeyShot's HDRI editor, rebuilt from their
//!   pins: the active one lights the scene, and each becomes a Studio;
//! * every camera, with its lens, orthographic frame and depth of field;
//! * the image style, the render's size and samples.
//!
//! Lengths stay in the file's millimetres here, and the import brings the
//! model and its setup into metres together. The floor is where KeyShot's
//! was, the scene's y = 0: a product lifted above it stays lifted, as it was
//! rendered there.
//!
//! What a scene only hints at is read as best the files themselves, and
//! KeyShot's own renders of them, support; the import's warnings say each
//! time a reading is a guess (`docs/keyshot-import.md` lists them).

use crate::bip::{self, Container, Item, Kind, Value};
use crate::keyshot::{self, Mtl, Shader, Val};
use crate::raw::{RawMesh, RawModelSet, RawMultiMaterial, RawNode, RawScene, RawSetup, RawTexture};
use crate::step::Tessellation;
use crate::ImportError;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use tequila_core::math::{vec2, vec3, Mat4, Transform, Vec2, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_core::units::LengthUnit;
use tequila_render_core::camera::{Aperture, Camera, Projection};
use tequila_render_core::pins::{self, GradientStop, Pin, PinBlend, PinKind, PinShape};
use tequila_scene::document::{BackgroundMode, EnvironmentSettings};
use tequila_scene::lighting::SavedCamera;
use tequila_scene::material::MaterialDefinition;
use tequila_scene::studio::Studio;

/// Visibility: bit 14 of a part's or a group's flags.
const VISIBLE: u32 = 0x4000;

/// A multi-material: bit 11 of a material's flags.
const MULTI: u32 = 0x0800;

/// Faces past this many control points, or trimming curves past this many,
/// are not a product's: a corrupt file's numbers, which would take the
/// divider all day.
const MOST_POINTS: usize = 250_000;

/// Where a scene's pictures are found.
pub(crate) enum Pictures<'a> {
    /// A package's own files, by their bare name in lower case.
    Package(&'a BTreeMap<String, Vec<u8>>),
    /// Beside the scene, in its folder.
    Folder(Option<&'a Path>),
    /// Beside a scene read from bytes: the reader's companion-file resolver,
    /// whose pictures are kept with the model as a package's are.
    Sibling(&'a dyn Fn(&str) -> Option<Vec<u8>>),
}

/// Whether the bytes are a KeyShot scene.
pub fn looks_like(bytes: &[u8]) -> bool {
    bip::looks_like(bytes)
}

/// A `.ksp`: the scene in it, with the pictures beside it.
pub(crate) fn read_package(bytes: &[u8], name: &str, quality: Tessellation) -> Result<RawScene, ImportError> {
    let members = crate::texture_set::read_zip(bytes)?;
    let scene = members
        .iter()
        .filter(|(n, d)| n.to_ascii_lowercase().ends_with(".bip") || bip::looks_like(d))
        .max_by_key(|(_, d)| d.len())
        .ok_or_else(|| ImportError::Malformed("the package holds no KeyShot scene (.bip)".into()))?;
    let files: BTreeMap<String, Vec<u8>> = members
        .iter()
        .filter(|(n, _)| !std::ptr::eq(n, &scene.0))
        .map(|(n, d)| (base_name(n).to_lowercase(), d.clone()))
        .collect();
    read(&scene.1, name, Pictures::Package(&files), quality)
}

/// Whether the bytes are a ZIP naming a KeyShot scene: what dividing a
/// `.ksp`'s model again reads. By the names in its directory, at its end,
/// rather than by inflating forty megabytes to look.
pub(crate) fn is_package(bytes: &[u8]) -> bool {
    let tail = &bytes[bytes.len().saturating_sub(1 << 20)..];
    bytes.starts_with(b"PK\x03\x04") && tail.windows(4).any(|w| w.eq_ignore_ascii_case(b".bip"))
}

fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The notes an import makes, each said once.
#[derive(Default)]
struct Notes(Vec<String>);

impl Notes {
    fn push(&mut self, note: impl Into<String>) {
        let note = note.into();
        if !self.0.contains(&note) {
            self.0.push(note);
        }
    }
}

/// The file's parts, found wherever they are.
#[derive(Default)]
struct File<'a> {
    shapes: HashMap<u32, &'a Container>,
    plugins: BTreeMap<String, &'a Container>,
    /// Project objects, by the number they are referred to by.
    objects: BTreeMap<u32, &'a Container>,
    /// The material library's shader nodes, one numbering for all of it.
    library: BTreeMap<u32, &'a Container>,
    /// Materials in file order.
    materials: Vec<&'a Container>,
    roots: Vec<&'a Container>,
    /// Each camera and the nodes of its own graph.
    cameras: Vec<(&'a Container, BTreeMap<u32, &'a Container>)>,
}

impl<'a> File<'a> {
    fn gather(&mut self, items: &'a [Item], pending: &mut BTreeMap<u32, &'a Container>) {
        for item in items {
            let Item::Container(c) = item else { continue };
            match c.kind {
                Kind::Chunk => self.gather(&c.children, pending),
                Kind::Shape => {
                    if let Some(id) = c.id() {
                        self.shapes.insert(id, c);
                    }
                }
                Kind::Plugin => {
                    self.plugins.insert(c.name.clone(), c);
                }
                Kind::Object => {
                    self.objects.insert(c.number, c);
                }
                // Nodes before the scene tree are the material library's;
                // after it, each camera's, coming before it.
                Kind::Node if self.roots.is_empty() => {
                    self.library.insert(c.number, c);
                }
                Kind::Node => {
                    pending.insert(c.number, c);
                }
                Kind::Material => self.materials.push(c),
                Kind::Group => self.roots.push(c),
                Kind::Camera => self.cameras.push((c, std::mem::take(pending))),
                _ => {}
            }
        }
    }

    /// The project objects of a class.
    fn of_class(&self, class: &str) -> impl Iterator<Item = &'a Container> + '_ {
        let class = class.to_string();
        self.objects.values().copied().filter(move |o| o.class == class)
    }

    fn settings(&self) -> Option<&'a Container> {
        self.plugins.get("luxrtopt.so").copied().or_else(|| self.plugins.values().copied().find(|p| p.value("render_width").is_some()))
    }
}

/// KeyShot's colours are as its colour picker shows them, sRGB-encoded:
/// a ColorChecker swatch picked in KeyShot is stored as the swatch's 8-bit
/// values over 255. Tequila's are linear.
fn linear(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn colour(c: [f32; 3]) -> Spectrum {
    Spectrum::from_linear_srgb(vec3(linear(c[0]), linear(c[1]), linear(c[2])))
}

/// A roughness as KeyShot stores it, as Tequila's. The stored numbers are
/// not round, so KeyShot may remap its slider before it stores it; read as
/// the square of a perceptual roughness, glossy black parts rendered satin
/// grey, where KeyShot's own render of the same scene shows them glossy.
/// Taken as they are, they agree with it, and with how the package
/// converter reads a material's. A best reading, not a measured one.
fn roughness(stored: f32) -> f32 {
    stored.clamp(0.0, 1.0)
}

/// A scene's shader node as the converter reads a package's.
fn shader(node: &Container) -> Shader {
    let mut params = Vec::new();
    for leaf in node.leaves() {
        let key = leaf.key.clone();
        let value = match &leaf.value {
            Value::F32(v) if matches!(key.as_str(), "roughness" | "metal_roughness") => Val::Numbers(vec![roughness(*v)]),
            Value::F32(v) => Val::Numbers(vec![*v]),
            Value::F64(v) => Val::Numbers(vec![*v as f32]),
            Value::I32(v) => Val::Numbers(vec![*v as f32]),
            Value::Bool(v) => Val::Flag(*v),
            // Every colour of a material node: linear, as the converter
            // expects.
            Value::Vec3f(c) => Val::Numbers(c.iter().map(|v| linear(*v)).collect()),
            Value::Vec4f(c) => Val::Numbers(vec![linear(c[0]), linear(c[1]), linear(c[2]), c[3]]),
            Value::Vec3d(v) => Val::Numbers(v.iter().map(|x| *x as f32).collect()),
            Value::Mat4d(m) => Val::Numbers(m.iter().map(|x| *x as f32).collect()),
            Value::Text(t) if key == "attribute" => continue,
            Value::Text(t) => Val::Text(t.clone()),
            // An input wired to a node is the converter's node by number
            // (`"shader" 3`); a parameter a texture drives, its link
            // (`"color" ! 3`).
            Value::Link(n) => Val::Numbers(vec![*n as f32]),
            Value::ParamLink(n) => Val::Link(*n),
            Value::I32s(v) => Val::List(v.iter().map(|n| *n as u32).collect()),
            _ => continue,
        };
        params.push((key, value));
    }
    Shader { kind: node.name.clone(), params }
}

/// A row-vector matrix (points multiplied on the left, translation in the
/// last row) as Tequila's.
fn transform(m: &[f64; 16]) -> Option<Transform> {
    let mut out = Mat4::IDENTITY;
    for (r, row) in out.m.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = m[c * 4 + r] as f32;
        }
    }
    Transform::from_matrix(out)
}

/// A shape's geometry, divided: in its CAD file's millimetres.
#[derive(Default)]
struct Piece {
    /// A divided NURBS face, which way it looks unknown; a mesh's is its
    /// own.
    nurbs: bool,
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    uvs: Vec<Vec2>,
    indices: Vec<u32>,
    edges: Vec<Vec3>,
}

/// Turn a body's faces to look the way KeyShot's own mesh of it looks.
///
/// A scene keeps no face's orientation: a face looks whichever way its
/// surface's parameters happen to run, and about half of a scene's faces
/// look into their bodies. Glass is coloured by how far light goes through
/// it, measured between where a ray goes in and where it comes out, so a
/// face looking the wrong way is a cover gone black. KeyShot's mesh of the
/// object was made knowing, so each face is turned to agree with the mesh
/// where it lies: the mesh triangle a point on the face lies over, among
/// those parallel to it, nearest its plane (a thin sheet's other side is
/// parallel too, just off it). Faces with no mesh over them are asked by
/// [`orient`]. Returns how many were turned.
fn orient_by(pieces: &mut [(bool, Piece)], reference: &[Piece]) -> usize {
    /// A mesh triangle, ready to ask whether a point lies over it.
    struct Triangle {
        a: Vec3,
        e1: Vec3,
        e2: Vec3,
        /// The mesh's own normal there.
        normal: Vec3,
        /// Its plane's normal, for the distance to it.
        plane: Vec3,
    }
    let mut triangles = Vec::new();
    for r in reference {
        for t in r.indices.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| r.positions[i as usize]);
            let Some(plane) = (b - a).cross(c - a).try_normalize() else { continue };
            // The mesh's own normals, as KeyShot shades with; its winding
            // where it has none.
            let shaded = if r.normals.len() == r.positions.len() { r.normals[t[0] as usize] + r.normals[t[1] as usize] + r.normals[t[2] as usize] } else { Vec3::ZERO };
            let normal = shaded.try_normalize().unwrap_or(plane);
            triangles.push(Triangle { a, e1: b - a, e2: c - a, normal, plane });
        }
    }
    if triangles.is_empty() {
        return orient(pieces);
    }
    let mut turned = 0;
    let mut unsure = Vec::new();
    for (k, (nurbs, piece)) in pieces.iter_mut().enumerate() {
        if !*nurbs {
            continue;
        }
        let Some((middle, normal)) = largest(piece) else { continue };
        // The parallel triangle the point lies over, nearest its plane: a
        // sheet's other side is parallel too, but further off.
        let mut best: Option<(f32, f32)> = None;
        for t in &triangles {
            let facing = normal.dot(t.normal);
            if facing.abs() < 0.7 {
                continue;
            }
            let off = (middle - t.a).dot(t.plane);
            if best.is_some_and(|(o, _)| off.abs() >= o) {
                continue;
            }
            let q = middle - t.plane * off - t.a;
            let (d00, d01, d11) = (t.e1.dot(t.e1), t.e1.dot(t.e2), t.e2.dot(t.e2));
            let denominator = d00 * d11 - d01 * d01;
            if denominator.abs() < 1e-20 {
                continue;
            }
            let (d20, d21) = (q.dot(t.e1), q.dot(t.e2));
            let s = (d11 * d20 - d01 * d21) / denominator;
            let r = (d00 * d21 - d01 * d20) / denominator;
            let slack = 0.02;
            if s < -slack || r < -slack || s + r > 1.0 + slack {
                continue;
            }
            if best.is_none_or(|(o, _)| off.abs() < o) {
                best = Some((off.abs(), facing));
            }
        }
        match best {
            Some((_, facing)) if facing < 0.0 => {
                turn(piece);
                turned += 1;
            }
            Some(_) => {}
            None => unsure.push(k),
        }
    }
    if !unsure.is_empty() {
        // Asked the other way, by rays: only the faces the mesh did not
        // answer for, with the rest as they now are.
        let mut asked: Vec<(bool, Piece)> = pieces.iter_mut().enumerate().map(|(k, (n, p))| (*n && unsure.contains(&k), std::mem::take(p))).collect();
        turned += orient(&mut asked);
        for ((_, p), (_, a)) in pieces.iter_mut().zip(asked) {
            *p = a;
        }
    }
    turned
}

/// The middle and normal (as wound) of a piece's largest triangle: a point
/// well inside the face, away from its edges.
fn largest(piece: &Piece) -> Option<(Vec3, Vec3)> {
    let t = piece.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]].map(|i| piece.positions[i as usize])).max_by(|a, b| {
        let area = |t: &[Vec3; 3]| (t[1] - t[0]).cross(t[2] - t[0]).length();
        area(a).total_cmp(&area(b))
    })?;
    let normal = (t[1] - t[0]).cross(t[2] - t[0]).try_normalize()?;
    Some(((t[0] + t[1] + t[2]) / 3.0, normal))
}

/// A piece turned over: its triangles wound the other way, its normals
/// reversed.
fn turn(piece: &mut Piece) {
    for t in piece.indices.chunks_exact_mut(3) {
        t.swap(1, 2);
    }
    for n in &mut piece.normals {
        *n = -*n;
    }
}

/// Turn a body's faces to look out of it, when there is no mesh to ask:
/// by rays from a point on each face along its normal. A ray that crosses
/// the body an odd number of times started inside it, and the face is
/// turned. Three rays a little apart vote, so one that grazes an edge does
/// not decide; a face of an open body, whose rays mostly leave without
/// crossing, stays as it is.
fn orient(pieces: &mut [(bool, Piece)]) -> usize {
    let triangles: Vec<[[f64; 3]; 3]> = pieces
        .iter()
        .flat_map(|(_, p)| p.indices.chunks_exact(3).map(move |t| [t[0], t[1], t[2]].map(|i| { let v = p.positions[i as usize]; [v.x as f64, v.y as f64, v.z as f64] })))
        .collect();
    if triangles.len() < 4 {
        return 0;
    }
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    // How many triangles a ray from `o` along `d` crosses, beyond `near`.
    let crossings = |o: [f64; 3], d: [f64; 3], near: f64| -> usize {
        let mut n = 0;
        for t in &triangles {
            let (e1, e2) = (sub(t[1], t[0]), sub(t[2], t[0]));
            let p = cross(d, e2);
            let det = dot(e1, p);
            if det.abs() < 1e-18 {
                continue;
            }
            let s = sub(o, t[0]);
            let u = dot(s, p) / det;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = cross(s, e1);
            let v = dot(d, q) / det;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            if dot(e2, q) / det > near {
                n += 1;
            }
        }
        n
    };
    let mut turned = 0;
    for (nurbs, piece) in pieces.iter_mut() {
        if !*nurbs {
            continue;
        }
        // The face's largest triangle: its middle, and its normal as wound.
        let Some(t) = piece
            .indices
            .chunks_exact(3)
            .map(|t| [t[0], t[1], t[2]].map(|i| { let v = piece.positions[i as usize]; [v.x as f64, v.y as f64, v.z as f64] }))
            .max_by(|a, b| {
                let area = |t: &[[f64; 3]; 3]| { let c = cross(sub(t[1], t[0]), sub(t[2], t[0])); dot(c, c) };
                area(a).total_cmp(&area(b))
            })
        else {
            continue;
        };
        let normal = cross(sub(t[1], t[0]), sub(t[2], t[0]));
        let length = dot(normal, normal).sqrt();
        if !(length > 0.0) {
            continue;
        }
        let n = normal.map(|c| c / length);
        let middle = [(t[0][0] + t[1][0] + t[2][0]) / 3.0, (t[0][1] + t[1][1] + t[2][1]) / 3.0, (t[0][2] + t[1][2] + t[2][2]) / 3.0];
        let near = 1e-6 * (length.sqrt()).max(1e-3);
        // Two directions across the normal, to lean the rays off it.
        let across = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
        let a = cross(n, across);
        let b = cross(n, a);
        let rays = [n, [n[0] + 0.07 * a[0], n[1] + 0.07 * a[1], n[2] + 0.07 * a[2]], [n[0] + 0.05 * b[0] - 0.04 * a[0], n[1] + 0.05 * b[1] - 0.04 * a[1], n[2] + 0.05 * b[2] - 0.04 * a[2]]];
        let inside = rays.iter().filter(|d| crossings(middle, **d, near) % 2 == 1).count();
        if inside >= 2 {
            turn(piece);
            turned += 1;
        }
    }
    turned
}

/// A trimmed NURBS face's numbers, checked against each other.
fn nurbs(g: &Container) -> Option<crate::iges::NurbsFace> {
    let order = |k: &str| usize::try_from(g.int(k)?).ok().filter(|o| (2..=32).contains(o));
    let (uo, vo) = (order("uorder")?, order("vorder")?);
    let (ku, kv) = (g.f64s("uknot")?.to_vec(), g.f64s("vknot")?.to_vec());
    let pw = g.f64s("Pw")?;
    if pw.len() % 4 != 0 || pw.len() / 4 > MOST_POINTS {
        return None;
    }
    let points: Vec<[f64; 4]> = pw.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
    let domain = [g.number("umin")?, g.number("umax")?, g.number("vmin")?, g.number("vmax")?];
    // The loops: so many edges each, each edge its own curve, the curves'
    // numbers end to end in four lists.
    let per_loop = g.i32s("trim_nedges").unwrap_or(&[]);
    let orders = g.i32s("edge_order").unwrap_or(&[]);
    let counts = g.i32s("edge_nknots").unwrap_or(&[]);
    let (from, to) = (g.f64s("edge_min").unwrap_or(&[]), g.f64s("edge_max").unwrap_or(&[]));
    let (knots, uvw) = (g.f64s("edge_knot").unwrap_or(&[]), g.f64s("edge_uvw").unwrap_or(&[]));
    let edges: usize = per_loop.iter().map(|&n| usize::try_from(n).ok()).sum::<Option<usize>>()?;
    if edges > MOST_POINTS || orders.len() != edges || counts.len() != edges || from.len() != edges || to.len() != edges {
        return None;
    }
    let (mut at_knot, mut at_point, mut edge) = (0usize, 0usize, 0usize);
    let mut loops = Vec::with_capacity(per_loop.len());
    for &n in per_loop {
        let mut curves = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let order = usize::try_from(orders[edge]).ok().filter(|o| (2..=32).contains(o))?;
            let count = usize::try_from(counts[edge]).ok()?;
            let points = count.checked_sub(order)?;
            let k = knots.get(at_knot..at_knot.checked_add(count)?)?.to_vec();
            let p = uvw.get(at_point..at_point.checked_add(points.checked_mul(3)?)?)?;
            curves.push(crate::iges::NurbsCurve {
                degree: order - 1,
                knots: k,
                points: p.chunks_exact(3).map(|q| [q[0], q[1], 0.0, q[2]]).collect(),
                range: [from[edge], to[edge]],
            });
            at_knot += count;
            at_point += points * 3;
            edge += 1;
        }
        loops.push(curves);
    }
    if at_knot != knots.len() || at_point != uvw.len() {
        return None;
    }
    Some(crate::iges::NurbsFace { degree: [uo - 1, vo - 1], knots: [ku, kv], points, domain, loops })
}

/// A shape divided at `tolerance` millimetres; `None` when it could not be.
fn divide(shape: &Container, tolerance: f64) -> Option<Piece> {
    let g = shape.containers().find(|c| c.kind == Kind::Geometry)?;
    match g.name.as_str() {
        "nurbs" => {
            let face = nurbs(g)?;
            let [u0, u1, v0, v1] = face.domain;
            let divided = crate::iges::divide_nurbs(face, tolerance)?;
            let to = |p: &[f64; 3]| vec3(p[0] as f32, p[1] as f32, p[2] as f32);
            let normals: Vec<Vec3> = divided.normals.iter().map(to).collect();
            Some(Piece {
                nurbs: true,
                positions: divided.positions.iter().map(to).collect(),
                normals,
                uvs: divided.params.iter().map(|q| vec2(((q[0] - u0) / (u1 - u0)) as f32, ((q[1] - v0) / (v1 - v0)) as f32)).collect(),
                indices: divided.triangles.iter().flatten().copied().collect(),
                edges: divided.edges.iter().map(to).collect(),
            })
        }
        "triangles" => {
            let vertices = g.f32s("vertices")?;
            let indices = g.i32s("triangles")?;
            if vertices.len() % 3 != 0 || indices.len() % 3 != 0 {
                return None;
            }
            let count = vertices.len() / 3;
            let indices: Vec<u32> = indices.iter().map(|&i| u32::try_from(i).ok().filter(|&i| (i as usize) < count)).collect::<Option<_>>()?;
            let positions: Vec<Vec3> = vertices.chunks_exact(3).map(|p| vec3(p[0], p[1], p[2])).collect();
            if !positions.iter().all(|p| p.is_finite()) {
                return None;
            }
            let normals = g.f32s("normals").filter(|n| n.len() == vertices.len()).map(|n| n.chunks_exact(3).map(|p| vec3(p[0], p[1], p[2])).collect()).unwrap_or_default();
            let uvs = g.pairs("uv_tex").filter(|t| t.len() == count).map(|t| t.iter().map(|p| vec2(p[0], p[1])).collect()).unwrap_or_default();
            Some(Piece { nurbs: false, positions, normals, uvs, indices, edges: Vec::new() })
        }
        _ => None,
    }
}

/// A shape's extent, from its control points or vertices, cheaply.
fn extent(shape: &Container) -> Option<([f64; 3], [f64; 3])> {
    let g = shape.containers().find(|c| c.kind == Kind::Geometry)?;
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    let mut grow = |p: [f64; 3]| {
        for k in 0..3 {
            if p[k].is_finite() {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    };
    if let Some(pw) = g.f64s("Pw") {
        for p in pw.chunks_exact(4).filter(|p| p[3] > 0.0) {
            grow([p[0] / p[3], p[1] / p[3], p[2] / p[3]]);
        }
    } else if let Some(v) = g.f32s("vertices") {
        for p in v.chunks_exact(3) {
            grow([p[0] as f64, p[1] as f64, p[2] as f64]);
        }
    }
    (lo[0] <= hi[0]).then_some((lo, hi))
}

/// Faces gathered into one part: an object's, of one material and
/// visibility.
struct Body {
    shapes: Vec<u32>,
    /// The KeyShot material it wears.
    material: Option<u32>,
    /// KeyShot's own mesh of the object the faces are of.
    reference: Vec<u32>,
}

/// A node of the tree being built: a [`RawNode`] waiting for its body.
struct Pending {
    node: RawNode,
    body: Option<usize>,
}

struct Builder<'a> {
    file: &'a File<'a>,
    nodes: Vec<Pending>,
    bodies: Vec<Body>,
    model_sets: Vec<RawModelSet>,
    notes: Notes,
    /// Source files of the models shown, to say when one is shown twice.
    shown_models: BTreeMap<String, usize>,
}

impl Builder<'_> {
    /// A group and what is under it. `set` is whether it is a model set's
    /// own group, which does not use the visibility bit.
    fn group(&mut self, g: &Container, parent: Option<usize>, set: bool) {
        let mut node = RawNode::group(if g.name.trim().is_empty() { "Group".to_string() } else { g.name.clone() });
        node.parent = parent;
        if let Some(m) = g.property("group_local_transform") {
            match transform(&m) {
                Some(t) => node.transform = t,
                None => self.notes.push(format!("{}'s placement could not be inverted; it was left where its file put it", g.name)),
            }
        }
        let visible = set || g.flags().is_some_and(|f| f & VISIBLE != 0);
        node.hidden = !visible;
        if let Some(file) = g.text("source file") {
            node.metadata.insert("KeyShot source file".into(), file.to_string());
            if visible && parent.is_some() {
                *self.shown_models.entry(file.to_string()).or_default() += 1;
            }
        }
        let index = self.nodes.len();
        self.nodes.push(Pending { node, body: None });
        if set {
            let name = g.text("model set").unwrap_or(&g.name).to_string();
            self.model_sets.push(RawModelSet { name, nodes: vec![index] });
        }

        // Its own faces, by the material they wear and whether they show.
        let mut parts: Vec<(u32, Option<u32>, bool, bool)> = Vec::new();
        let mut groups = 0;
        for child in g.containers() {
            match child.kind {
                Kind::Group => {
                    groups += 1;
                    self.group(child, Some(index), false);
                }
                Kind::Part => {
                    let Some(shape) = child.leaves().find_map(|l| match l.value {
                        Value::ShapeRef(s) => Some(s),
                        _ => None,
                    }) else {
                        self.notes.push("a part that names no geometry was left out");
                        continue;
                    };
                    let material = child.leaves().find_map(|l| match l.value {
                        Value::MaterialRef(m) => Some(m),
                        _ => None,
                    });
                    let shows = child.flags().is_some_and(|f| f & VISIBLE != 0);
                    let mesh = self.file.shapes.get(&shape).and_then(|s| s.containers().find(|c| c.kind == Kind::Geometry)).is_some_and(|g| g.name == "triangles");
                    parts.push((shape, material, shows, mesh));
                }
                _ => {}
            }
        }
        // A CAD object comes with its faces and a mesh KeyShot made of all
        // of them, the one it renders: the same object twice. The faces are
        // what is divided, at the import's quality; the mesh says which way
        // each face looks, which the faces do not, and stands in for the
        // object if they cannot be divided. An object that is only a mesh
        // is its mesh.
        let has_faces = parts.iter().any(|p| !p.3);
        let reference: Vec<u32> = if has_faces { parts.iter().filter(|p| p.3).map(|p| p.0).collect() } else { Vec::new() };
        let mut bodies: Vec<(Option<u32>, bool, Vec<u32>)> = Vec::new();
        for &(shape, material, shows, mesh) in &parts {
            if has_faces && mesh {
                continue;
            }
            match bodies.iter_mut().find(|(m, s, _)| *m == material && *s == shows) {
                Some((_, _, shapes)) => shapes.push(shape),
                None => bodies.push((material, shows, vec![shape])),
            }
        }
        // An object of one material is one part, under its own name; an
        // object of several keeps its name on a group of them.
        if groups == 0 && bodies.len() == 1 {
            let (material, shows, shapes) = bodies.pop().expect("one");
            self.nodes[index].body = Some(self.bodies.len());
            if !shows {
                self.nodes[index].node.hidden = true;
            }
            self.bodies.push(Body { shapes, material, reference });
            return;
        }
        let several = bodies.len() > 1;
        for (k, (material, shows, shapes)) in bodies.into_iter().enumerate() {
            let label = material
                .and_then(|m| self.file.materials.iter().find(|c| c.id() == Some(m)))
                .map(|c| c.name.clone())
                .filter(|n| several && !n.trim().is_empty())
                .map(|n| format!("{} ({n})", g.name))
                .unwrap_or_else(|| if several { format!("{} {}", g.name, k + 1) } else { g.name.clone() });
            let mut part = RawNode::group(label);
            part.parent = Some(index);
            part.hidden = !shows;
            self.nodes.push(Pending { node: part, body: Some(self.bodies.len()) });
            self.bodies.push(Body { shapes, material, reference: reference.clone() });
        }
    }
}

/// Read a KeyShot scene, dividing its faces at `quality`; its pictures are
/// found as `pictures` says.
pub(crate) fn read(bytes: &[u8], name: &str, pictures: Pictures, quality: Tessellation) -> Result<RawScene, ImportError> {
    let parsed = bip::parse(bytes).map_err(|e| ImportError::Malformed(format!("this is not a KeyShot scene Tequila can read: {e}")))?;
    let mut file = File::default();
    let mut pending = BTreeMap::new();
    file.gather(&parsed.items, &mut pending);
    if file.roots.is_empty() && file.shapes.is_empty() {
        return Err(ImportError::Malformed(format!(
            "the KeyShot scene holds no models{}",
            parsed.warnings.first().map(|w| format!(" ({w})")).unwrap_or_default()
        )));
    }

    let mut builder = Builder { file: &file, nodes: Vec::new(), bodies: Vec::new(), model_sets: Vec::new(), notes: Notes::default(), shown_models: BTreeMap::new() };
    for w in &parsed.warnings {
        builder.notes.push(w.clone());
    }
    let mut roots = Vec::new();
    for root in &file.roots {
        let at = builder.nodes.len();
        let set = root.text("model set").is_some();
        roots.push(at);
        builder.group(root, None, set);
        // A model set's group is named for the set ("Default"), which the
        // Model Sets list keeps; in the tree it is the file's model.
        if set && file.roots.len() == 1 && !name.trim().is_empty() {
            builder.nodes[at].node.name = name.to_string();
        }
    }
    for (_, count) in builder.shown_models.iter().filter(|(_, n)| **n > 1) {
        builder.notes.push(format!("{count} models shown at once come from the same CAD file and stand in the same place, as they did in KeyShot"));
    }

    // The faces, divided side by side. A body's tolerance is its own size
    // at the quality's share, as a STEP or IGES body's is, but never finer
    // than a tenth of that share of the whole model.
    let started = std::time::Instant::now();
    let boxes: Vec<([f64; 3], [f64; 3])> = builder
        .bodies
        .iter()
        .map(|b| {
            let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
            for e in b.shapes.iter().filter_map(|s| file.shapes.get(s)).filter_map(|s| extent(s)) {
                for k in 0..3 {
                    lo[k] = lo[k].min(e.0[k]);
                    hi[k] = hi[k].max(e.1[k]);
                }
            }
            (lo, hi)
        })
        .collect();
    let size = |(lo, hi): &([f64; 3], [f64; 3])| if lo[0] <= hi[0] { ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt() } else { 0.0 };
    let model = {
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for b in boxes.iter().filter(|b| b.0[0] <= b.1[0]) {
            for k in 0..3 {
                lo[k] = lo[k].min(b.0[k]);
                hi[k] = hi[k].max(b.1[k]);
            }
        }
        size(&(lo, hi)).max(1e-6)
    };
    let tolerances: Vec<f64> = boxes.iter().map(|b| (size(b) * quality.chord()).max(model * quality.chord() * 0.1)).collect();
    // Each body's faces, and KeyShot's mesh of its object beside them.
    let jobs: Vec<(usize, u32, bool)> = builder
        .bodies
        .iter()
        .enumerate()
        .flat_map(|(i, b)| b.shapes.iter().map(move |&s| (i, s, false)).chain(b.reference.iter().map(move |&s| (i, s, true))))
        .collect();
    let divided: Vec<Option<Piece>> = {
        use rayon::prelude::*;
        jobs.par_iter().map(|&(body, shape, _)| file.shapes.get(&shape).and_then(|s| divide(s, tolerances[body]))).collect()
    };
    let faces = jobs.iter().filter(|j| !j.2).count();
    let mut missing = 0usize;
    let mut unknown = 0usize;
    let mut pieces: Vec<Vec<(bool, Piece)>> = (0..builder.bodies.len()).map(|_| Vec::new()).collect();
    let mut references: Vec<Vec<Piece>> = (0..builder.bodies.len()).map(|_| Vec::new()).collect();
    let mut lost = vec![false; builder.bodies.len()];
    for ((body, shape, reference), piece) in jobs.iter().zip(divided) {
        match (piece, reference) {
            (Some(piece), true) => references[*body].push(piece),
            (Some(piece), false) => pieces[*body].push((piece.nurbs, piece)),
            (None, true) => {}
            (None, false) if file.shapes.contains_key(shape) => {
                missing += 1;
                lost[*body] = true;
            }
            (None, false) => unknown += 1,
        }
    }
    let turned: usize = {
        use rayon::prelude::*;
        pieces.par_iter_mut().zip(references.par_iter()).map(|(p, r)| orient_by(p, r)).sum()
    };
    // A body that lost faces is KeyShot's mesh of it instead, where the
    // mesh is its alone; its faces' edges are kept.
    let mut stood_in = 0usize;
    for (i, body) in builder.bodies.iter().enumerate() {
        let shared = builder.bodies.iter().filter(|b| !b.reference.is_empty() && b.reference == body.reference).count() > 1;
        if lost[i] && !references[i].is_empty() && !shared {
            let edges: Vec<Vec3> = pieces[i].iter().flat_map(|(_, p)| p.edges.iter().copied()).collect();
            pieces[i] = std::mem::take(&mut references[i]).into_iter().map(|p| (false, p)).collect();
            if let Some((_, first)) = pieces[i].first_mut() {
                first.edges = edges;
            }
            stood_in += 1;
        }
    }
    if stood_in > 0 {
        builder.notes.push(format!("{stood_in} objects whose faces could not all be divided are KeyShot's own mesh of them instead"));
    }
    let mut meshes: Vec<Option<RawMesh>> = (0..builder.bodies.len()).map(|_| None).collect();
    // Whether every piece of a body came with normals: one without them
    // and the body's are all made by the pipeline, smoothly, rather than
    // some left zero.
    let mut normals = vec![true; builder.bodies.len()];
    for (body, list) in pieces.into_iter().enumerate() {
        for (_, piece) in list {
            let mesh = meshes[body].get_or_insert_with(RawMesh::default);
            let had = mesh.positions.len();
            mesh.positions.extend(&piece.positions);
            if piece.normals.len() == piece.positions.len() {
                mesh.normals.extend(&piece.normals);
            } else {
                normals[body] = false;
            }
            // UVs where a piece has them, zero where it has none.
            if piece.uvs.len() == piece.positions.len() {
                mesh.uvs.resize(had, Vec2::ZERO);
                mesh.uvs.extend(&piece.uvs);
            } else if !mesh.uvs.is_empty() {
                mesh.uvs.resize(mesh.positions.len(), Vec2::ZERO);
            }
            mesh.indices.extend(piece.indices.iter().map(|i| i + had as u32));
            mesh.edge_lines.extend(&piece.edges);
        }
    }
    for (mesh, complete) in meshes.iter_mut().zip(&normals) {
        if let Some(mesh) = mesh.as_mut().filter(|_| !complete) {
            mesh.normals.clear();
        }
    }
    let elapsed = started.elapsed();
    if turned > 0 {
        builder.notes.push(format!("{turned} faces were turned to look out of their bodies: the scene does not keep which way a face looks"));
    }
    if missing > 0 {
        builder.notes.push(format!("{missing} of {faces} faces could not be divided and are missing"));
    }
    if unknown > 0 {
        builder.notes.push(format!("{unknown} parts name geometry the scene does not hold"));
    }
    if file.shapes.values().any(|s| s.containers().any(|g| g.f32("rounded_edge_radius").is_some_and(|r| r > 0.0))) {
        builder.notes.push("KeyShot's rounded edges on meshes have no counterpart; their edges are sharp");
    }

    // The materials, then each body's.
    let mut scene = RawScene { source_unit: Some(LengthUnit::Millimetre), tessellation: Some(quality.label().to_string()), ..RawScene::default() };
    let scratch = std::env::temp_dir().join(format!(
        "tequila-ksp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
    ));
    let materials = materials(&file, &pictures, &scratch, &mut scene, &mut builder.notes);
    let _ = std::fs::remove_dir_all(&scratch);
    let materials = materials?;

    let Builder { nodes, bodies, model_sets, mut notes, .. } = builder;
    let mut mesh_of: Vec<Option<usize>> = vec![None; bodies.len()];
    let mut multi_parts: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (i, (body, mesh)) in bodies.iter().zip(meshes).enumerate() {
        let Some(mut mesh) = mesh.filter(|m| !m.indices.is_empty()) else { continue };
        if !mesh.normals.is_empty() && mesh.normals.len() != mesh.positions.len() {
            mesh.normals.clear();
        }
        mesh.material = match body.material {
            Some(id) => match materials.single.get(&id) {
                Some(&index) => Some(index),
                None => match materials.multi.get(&id) {
                    Some(multi) => multi.variants.get(multi.active).copied(),
                    None => {
                        notes.push("some parts wear a material the scene does not hold; they came in with the default");
                        None
                    }
                },
            },
            None => None,
        };
        mesh.name = nodes.iter().find(|p| p.body == Some(i)).map(|p| p.node.name.clone()).unwrap_or_default();
        mesh_of[i] = Some(scene.meshes.len());
        scene.meshes.push(mesh);
    }
    for (index, pending) in nodes.into_iter().enumerate() {
        let mut node = pending.node;
        node.mesh = pending.body.and_then(|b| mesh_of[b]);
        if let Some(id) = pending.body.and_then(|b| bodies[b].material) {
            if materials.multi.contains_key(&id) && node.mesh.is_some() {
                multi_parts.entry(id).or_default().push(index);
            }
        }
        scene.nodes.push(node);
    }
    scene.roots = roots;
    if scene.meshes.is_empty() {
        return Err(ImportError::Malformed(format!(
            "no face of the KeyShot scene could be divided{}",
            notes.0.first().map(|w| format!(" ({w})")).unwrap_or_default()
        )));
    }

    let mut setup = RawSetup { model_sets, ..RawSetup::default() };
    let settings = file.settings();
    if let Some(active) = settings.and_then(|s| s.text("active_scene_set")) {
        setup.active_model_set = setup.model_sets.iter().position(|s| s.name == active);
    }
    for (id, multi) in &materials.multi {
        setup.multi_materials.push(RawMultiMaterial {
            name: multi.name.clone(),
            variants: multi.variants.clone(),
            active: multi.active,
            parts: multi_parts.remove(id).unwrap_or_default(),
        });
    }
    staging(&file, &mut setup, &mut notes);
    notes.push(format!("{faces} faces divided in {:.1} s at {} quality", elapsed.as_secs_f64(), quality.label()));
    scene.setup = Some(Box::new(setup));
    scene.warnings = notes.0;
    scene.validate().map_err(ImportError::Malformed)?;
    Ok(scene)
}

/// A multi-material: its variants by their place in the scene's
/// materials, and which is shown.
struct Multi {
    name: String,
    variants: Vec<usize>,
    active: usize,
}

struct Materials {
    /// KeyShot material id to its place in the scene's materials.
    single: BTreeMap<u32, usize>,
    multi: BTreeMap<u32, Multi>,
}

/// The scene's materials converted, their pictures found.
fn materials(file: &File, pictures: &Pictures, scratch: &Path, scene: &mut RawScene, notes: &mut Notes) -> Result<Materials, ImportError> {
    let library: BTreeMap<u32, Shader> = file.library.iter().map(|(&n, c)| (n, shader(c))).collect();
    // The pictures, each written out once.
    let mut found: BTreeMap<String, Option<PathBuf>> = BTreeMap::new();
    let mut picture = |wanted: &str| -> Result<Option<PathBuf>, String> {
        let name = base_name(wanted).to_string();
        let key = name.to_lowercase();
        if let Some(path) = found.get(&key) {
            return Ok(path.clone());
        }
        let relative = wanted.trim_start_matches(".\\").trim_start_matches("./").replace('\\', "/");
        // A picture in hand, written where the converter can read it and
        // the import can keep it.
        let keep = |bytes: &[u8]| -> Result<PathBuf, String> {
            std::fs::create_dir_all(scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
            let safe: String = name.chars().map(|c| if c.is_alphanumeric() || " ._-".contains(c) { c } else { '_' }).collect();
            let path = scratch.join(if safe.is_empty() { "texture".to_string() } else { safe });
            std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(path)
        };
        let path = match pictures {
            Pictures::Package(files) => files.get(&key).map(|bytes| keep(bytes)).transpose()?,
            Pictures::Sibling(sibling) => sibling(&relative).or_else(|| sibling(&name)).map(|bytes| keep(&bytes)).transpose()?,
            // Beside the scene, as written or by its bare name; never out
            // of its folder, whatever the scene says.
            Pictures::Folder(Some(folder)) => {
                let root = folder.canonicalize().ok();
                [folder.join(&relative), folder.join(&name)]
                    .into_iter()
                    .filter_map(|p| p.canonicalize().ok())
                    .find(|p| p.is_file() && root.as_ref().is_some_and(|r| p.starts_with(r)))
            }
            Pictures::Folder(None) => None,
        };
        found.insert(key, path.clone());
        Ok(path)
    };

    let mut out = Materials { single: BTreeMap::new(), multi: BTreeMap::new() };
    let mut multis = Vec::new();
    for m in &file.materials {
        let Some(id) = m.id() else {
            notes.push(format!("the material {} has no number, so nothing can wear it; it was left out", m.name));
            continue;
        };
        let list = m.i32s("materials");
        if m.flags().is_some_and(|f| f & MULTI != 0) || list.is_some_and(|l| !l.is_empty()) {
            multis.push((id, *m));
            continue;
        }
        let root = m.leaves().find_map(|l| match l.value {
            Value::MatRoot(_, root) => Some(root),
            _ => None,
        });
        let mtl = Mtl { name: m.name.clone(), shaders: library.clone(), surface: root, geometry: None, icon: None };
        let definition = match root.filter(|r| library.contains_key(r)) {
            Some(_) => match keyshot::convert_scene(&mtl, &mut picture, scratch) {
                Ok(converted) => {
                    for note in converted.notes {
                        notes.push(note);
                    }
                    let mut graph = converted.entry.graph;
                    // Its pictures into the scene's list, numbered from 0.
                    let mut renumbered = BTreeMap::new();
                    for (local, source) in &converted.entry.textures {
                        let in_scratch = source.path.starts_with(scratch);
                        let raw = if in_scratch {
                            let bytes = std::fs::read(&source.path).map_err(|e| ImportError::Io(format!("{}: {e}", source.path.display())))?;
                            let uri = source.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| "texture".into());
                            RawTexture { uri, role: source.role, embedded: Some(std::sync::Arc::new(bytes)) }
                        } else {
                            RawTexture { uri: source.path.display().to_string(), role: source.role, embedded: None }
                        };
                        let at = match scene.textures.iter().position(|t| t.same_image(&raw)) {
                            Some(at) => at,
                            None => {
                                scene.textures.push(raw);
                                scene.textures.len() - 1
                            }
                        };
                        renumbered.insert(*local, tequila_materials::graph::TextureId(at as u32));
                    }
                    graph.remap_textures(&|local| renumbered.get(&local).copied().unwrap_or(local));
                    MaterialDefinition::from_graph(m.name.clone(), converted.entry.material_type, graph)
                }
                Err(e) => {
                    notes.push(format!("the material {} could not be converted ({e}); it came in as grey plastic", m.name));
                    MaterialDefinition::plastic(m.name.clone())
                }
            },
            None => {
                notes.push(format!("the material {} has no surface; it came in as grey plastic", m.name));
                MaterialDefinition::plastic(m.name.clone())
            }
        };
        out.single.insert(id, scene.materials.len());
        scene.materials.push(definition);
    }
    for (id, m) in multis {
        let ids: Vec<u32> = m.i32s("materials").unwrap_or(&[]).iter().map(|&n| n as u32).collect();
        let variants: Vec<usize> = ids.iter().filter_map(|n| out.single.get(n).copied()).collect();
        if variants.len() < ids.len() {
            notes.push(format!("the multi-material {} lists variants the scene does not hold; they were left out", m.name));
        }
        if variants.is_empty() {
            notes.push(format!("the multi-material {} has no variants; what wears it came in with the default", m.name));
            continue;
        }
        // The variant shown is the one its root is the root of.
        let root = m.leaves().find_map(|l| match l.value {
            Value::MatRoot(_, root) => Some(root),
            _ => None,
        });
        let active = ids
            .iter()
            .filter(|n| out.single.contains_key(n))
            .position(|&n| {
                let sub = file.materials.iter().find(|c| c.id() == Some(n));
                sub.and_then(|c| c.leaves().find_map(|l| match l.value {
                    Value::MatRoot(_, r) => Some(r),
                    _ => None,
                })) == root
            })
            .unwrap_or(0);
        out.multi.insert(id, Multi { name: m.name.clone(), variants, active });
    }
    Ok(out)
}

/// The scene's staging: its cameras, environments, image style, render
/// settings, and the Studios that keep its other shots.
fn staging(file: &File, setup: &mut RawSetup, notes: &mut Notes) {
    let settings = file.settings();
    let height = settings.and_then(|s| s.f32("render_height")).filter(|h| *h > 0.0).unwrap_or(1080.0);

    // The image style the render uses, else the active one.
    let styles: Vec<&Container> = file.objects.values().copied().filter(|o| o.class.starts_with("lux.style.")).collect();
    let chosen = settings.and_then(|s| s.int("output_style_id")).and_then(|id| styles.iter().find(|o| o.id() == Some(id as u32)).copied());
    let style = chosen.or_else(|| styles.iter().find(|o| o.flag("active") == Some(true)).copied()).or_else(|| styles.first().copied());
    let mut backdrop = None;
    if let Some(style) = style {
        let (look, background) = image_style(style, height, notes);
        setup.image_style = Some(look);
        backdrop = background;
    }

    // Environments, in the scene's list; the active one lights the scene.
    let environments: Vec<(&Container, EnvironmentSettings)> = file.of_class("lux.env.hdri").map(|o| (o, environment(file, o, backdrop, notes))).collect();
    if let Some((_, active)) = environments.iter().find(|(o, _)| o.flag("active") == Some(true)).or_else(|| environments.first()) {
        setup.environment = Some(active.clone());
    }
    for class in file.objects.values().map(|o| o.class.as_str()).filter(|c| c.starts_with("lux.env.") && (c.contains("sun") || c.contains("sky") || c.contains("light"))) {
        notes.push(format!("a KeyShot environment object of the kind `{class}` has no counterpart and was left out"));
    }
    if environments.is_empty() {
        if let Some(texture) = file.cameras.iter().flat_map(|(_, g)| g.values()).find(|n| n.name == "lux_envmap").and_then(|n| n.text("texture")) {
            notes.push(format!("the scene is lit by the picture {}, which a scene does not carry; it came in with Tequila's studio", base_name(texture)));
        }
    }

    // The cameras; the view the scene was saved in is the active one.
    for (c, graph) in &file.cameras {
        let Some(camera) = camera(c, graph, notes) else { continue };
        if c.name == "last_active" && c.id().is_none() {
            setup.active_camera = Some(setup.cameras.len());
        }
        setup.cameras.push(camera);
    }
    if setup.active_camera.is_none() && !setup.cameras.is_empty() {
        setup.active_camera = Some(0);
    }

    // KeyShot keeps environments and cameras apart, and these scenes name
    // each environment after the camera it was made for: each named camera
    // is a Studio with the environment of its name, where there is one, and
    // each environment left over is a Studio of its own.
    let key = |name: &str| name.trim().to_lowercase();
    let mut used = vec![false; environments.len()];
    for (camera, (c, _)) in setup.cameras.iter().zip(file.cameras.iter().filter(|(c, g)| camera_ok(c, g))) {
        if c.name == "last_active" && c.id().is_none() {
            continue;
        }
        let found = environments.iter().position(|(o, _)| key(&o.name) == key(&c.name));
        if let Some(i) = found {
            used[i] = true;
        }
        setup.studios.push(Studio { name: c.name.clone(), camera: Some(camera.clone()), environment: found.map(|i| environments[i].1.clone()), ..Studio::default() });
    }
    for ((o, settings), used) in environments.iter().zip(used) {
        if !used {
            setup.studios.push(Studio { name: o.name.clone(), environment: Some(settings.clone()), ..Studio::default() });
        }
    }
    if !setup.studios.is_empty() {
        notes.push("each KeyShot camera became a Studio, with the environment named after it where there is one; environments no camera is named after are Studios of their own");
    }

    // The render: its size and samples.
    if let Some(s) = settings {
        let mut render = tequila_scene::render::RenderSettings::default();
        if let (Some(w), Some(h)) = (s.int("render_width").filter(|w| *w > 0), s.int("render_height").filter(|h| *h > 0)) {
            render.width = w as usize;
            render.height = h as usize;
        }
        if let Some(samples) = s.int("render_samples").filter(|n| *n > 0) {
            render.max_samples = samples as u32;
        }
        if let Some(dpi) = s.f32("render_dpi").filter(|d| *d > 0.0) {
            render.dpi = dpi;
        }
        if let Some(style) = style {
            render.denoise = style.flag("denoise").unwrap_or(render.denoise);
            if let Some(blend) = style.f32("denoise_blend") {
                render.denoise_strength = blend.clamp(0.0, 1.0);
            }
        }
        setup.render = Some(render);
    }
}

/// Whether a camera reads: what [`camera`] needs.
fn camera_ok(c: &Container, _graph: &BTreeMap<u32, &Container>) -> bool {
    c.vec3d("position").is_some() && c.vec3d("direction").is_some()
}

/// A camera: where it is, which way it looks, its lens and its focus.
/// KeyShot's field of view is horizontal, across a 36 mm film.
fn camera(c: &Container, graph: &BTreeMap<u32, &Container>, notes: &mut Notes) -> Option<SavedCamera> {
    if !camera_ok(c, graph) {
        notes.push(format!("the camera {} has no position or direction and was left out", c.name));
        return None;
    }
    let v = |p: [f64; 3]| vec3(p[0] as f32, p[1] as f32, p[2] as f32);
    let position = v(c.vec3d("position")?);
    let direction = v(c.vec3d("direction")?).try_normalize().unwrap_or(-Vec3::Z);
    let up = c.vec3d("up").map(v).and_then(|u| u.try_normalize()).unwrap_or(Vec3::Y);
    let pivot = c.vec3d("pivot").filter(|_| c.flag("has_pivot") != Some(false)).map(v);
    let distance = pivot.map(|p| (p - position).dot(direction)).filter(|d| *d > 0.0).unwrap_or(1000.0);
    let mut camera = Camera {
        camera_to_world: Mat4::look_at(position, position + direction, up),
        focus_distance: distance,
        ..Camera::default()
    };
    let fov = c.f32("fov").filter(|f| *f > 0.01 && *f < 179.0).unwrap_or(39.6);
    let focal = 18.0 / (fov.to_radians() * 0.5).tan();
    camera.projection = if c.text("projection") == Some("orthographic") {
        match c.vec4f("window") {
            Some([_, _, bottom, top]) if top > bottom => Projection::Orthographic { scale: top - bottom },
            _ => Projection::Orthographic { scale: 2.0 * distance * (fov.to_radians() * 0.5).tan() * 9.0 / 16.0 },
        }
    } else {
        Projection::Perspective { focal_length_mm: focal }
    };
    // Depth of field, from the lens node the camera names.
    if let Some(lens) = c.link("lens").and_then(|n| graph.get(&n)).filter(|n| n.name == "rtrt_dof_effect") {
        if let Some(stop) = lens.f32("fstop").filter(|s| *s > 0.0) {
            camera.f_number = stop;
        }
        if let Some(focus) = lens.f32("focal_dist").filter(|d| *d > 0.0) {
            camera.focus_distance = focus;
        }
        let blades = lens.int("blades").unwrap_or(0);
        if blades >= 3 {
            camera.aperture = Aperture::Bladed { blades: blades as u32, rotation: lens.f32("shutter_offset").unwrap_or(0.0) };
        }
    }
    let target = pivot.unwrap_or(position + direction * distance);
    let name = if c.name == "last_active" && c.id().is_none() { "Viewport".to_string() } else { c.name.clone() };
    Some(SavedCamera { name, camera, target })
}

/// The image style: KeyShot's Photographic style onto Tequila's, and the
/// colour it puts behind the product when it has one.
fn image_style(o: &Container, render_height: f32, notes: &mut Notes) -> (tequila_color::ImageStyle, Option<Spectrum>) {
    use tequila_color::{ImageStyle, ToneMode};
    let mut style = ImageStyle { mode: if o.class == "lux.style.filmic" { ToneMode::Photographic } else { ToneMode::Basic }, ..ImageStyle::default() };
    let f = |k: &str| o.f32(k).unwrap_or(0.0);
    let on = |k: &str| o.flag(k).unwrap_or(false);
    style.exposure = f("exposure");
    // KeyShot's contrast and saturation are offsets from 0; Tequila's are
    // factors about 1.
    style.contrast = (1.0 + f("contrast")).max(0.0);
    if on("vibsat_enabled") {
        style.saturation = (1.0 + f("saturation")).max(0.0);
        if f("vibrance") != 0.0 {
            notes.push("the image style's vibrance has no counterpart; its saturation came over");
        }
    }
    if on("curve_enabled") {
        style.shadows = f("curve_shadow").clamp(-1.0, 1.0);
        style.highlights = f("curve_highlight").clamp(-1.0, 1.0);
        style.whites = f("curve_white").clamp(-1.0, 1.0);
        if f("curve_midtone") != 0.0 || f("curve_light") != 0.0 {
            notes.push("the image style's curve came over as shadows, highlights and whites; its midtone and light points have no counterpart");
        }
    }
    if f("white_balance") != 0.0 {
        notes.push("the image style's white balance is a KeyShot setting with no known scale; it was left neutral");
    }
    if let Some(mode) = o.int("tonemapping") {
        notes.push(format!("the image style's tone mapping ({mode}) is not understood; Tequila's own display transform was kept"));
    }
    if on("bloom") {
        style.bloom = f("bloom_strength").clamp(0.0, 1.0);
        style.bloom_radius = (f("bloom_radius") / render_height).clamp(0.001, 0.5);
        if f("bloom_threshold") != 0.0 {
            notes.push("the image style's bloom threshold has no counterpart");
        }
    }
    if on("vignette") {
        style.vignette = f("vignette_strength").clamp(0.0, 1.0);
    }
    if on("chromatic") {
        style.chromatic_aberration = f("chromatic_strength").max(0.0) * 0.01;
    }
    if on("frontplate") {
        notes.push("the image style's front plate was left out");
    }
    let background = (o.flag("use_background_color") == Some(true)).then(|| o.vec3f("background_color").map(colour).unwrap_or(Spectrum::ONE));
    (style, background)
}

/// A brush's colour: its colour times its colorize tint, raised to its
/// gamma. Hue and saturation shifts are said and left out.
fn brush_colour(brush: &Container, notes: &mut Notes) -> Spectrum {
    let base = brush.vec3f("color").map(colour).unwrap_or(Spectrum::ONE);
    let tint = brush.vec3f("color.colorize").map(colour).unwrap_or(Spectrum::ONE);
    let gamma = brush.f32("color.gamma").filter(|g| *g > 0.0).unwrap_or(1.0);
    let mut c = base * tint;
    if (gamma - 1.0).abs() > 1e-4 {
        c = Spectrum { r: c.r.max(0.0).powf(gamma), g: c.g.max(0.0).powf(gamma), b: c.b.max(0.0).powf(gamma) };
    }
    if brush.f32("color.hue").is_some_and(|h| h != 0.0) || brush.f32("color.saturation").is_some_and(|s| s != 0.0) {
        notes.push("an HDRI-editor colour's hue and saturation shifts were left out");
    }
    c
}

/// An HDRI-editor environment: its background colour over the whole
/// sphere and its pins over that, bottom to top. KeyShot keeps no picture
/// of it; it is rebuilt from the description, as KeyShot rebuilds it.
fn environment(file: &File, o: &Container, backdrop: Option<Spectrum>, notes: &mut Notes) -> EnvironmentSettings {
    let mut e = EnvironmentSettings { brightness: o.f32("map.gain").unwrap_or(1.0).max(0.0), contrast: o.f32("map.gamma").filter(|g| *g > 0.0).unwrap_or(1.0), ..EnvironmentSettings::default() };
    // The dome: KeyShot's size is its radius in metres (the renderer's node
    // says radius, at a thousand times), and its height is taken as a share
    // of it. Lengths in the file's millimetres.
    let radius = o.f32("map.size").filter(|s| *s > 0.0).unwrap_or(0.0) * 1000.0;
    e.size = 2.0 * radius;
    e.height = o.f32("map.height").unwrap_or(0.0) * radius;
    e.rotation = o.f32("map.rotation").unwrap_or(0.0).to_radians();
    e.ground.shadows = o.flag("ground.shadows").unwrap_or(true);
    e.ground.occlusion_shadows = o.flag("ground.occlusionShadows").unwrap_or(false);
    if o.flag("ground.reflection") == Some(true) {
        e.ground.reflections = 1.0;
        e.ground.reflection_fresnel = true;
    }
    e.ground.flatten = o.flag("ground.flatten").unwrap_or(false);
    e.ground.size = o.f32("ground.size").filter(|s| *s > 0.0).unwrap_or(0.0) * 2000.0;
    if let Some(c) = o.vec3f("ground.shadowColor") {
        e.ground.shadow_color = colour(c);
    }
    match (backdrop, o.int("background.type").unwrap_or(0)) {
        (Some(c), _) => {
            e.background = BackgroundMode::Color;
            e.background_color = c;
        }
        (None, 1) => {
            e.background = BackgroundMode::Color;
            e.background_color = o.vec3f("background.color").map(colour).unwrap_or(Spectrum::ONE);
        }
        (None, 2) => notes.push(format!("{}'s backplate picture is not in the scene; the environment shows behind the product instead", o.name)),
        _ => {}
    }

    let Some(map) = o.object_link("map").and_then(|n| file.objects.get(&n)) else {
        notes.push(format!("the environment {} is not one made in the HDRI editor; it came in with Tequila's studio", o.name));
        return e;
    };
    // The background fills the sphere and lights with it: a gradient of one
    // colour over all of it, laid over Tequila's studio so none of that
    // shows.
    if let Some(brush) = map.object_link("background").and_then(|n| file.objects.get(&n)) {
        let gain = brush.f32("color.gain").unwrap_or(1.0).max(0.0);
        e.pins.push(Pin {
            name: "Background".into(),
            kind: PinKind::Gradient { stops: vec![GradientStop { t: 0.0, color: Spectrum::ONE }, GradientStop { t: 1.0, color: Spectrum::ONE }], radial: true },
            direction: -Vec3::Y,
            angular_size: Vec2 { x: std::f32::consts::TAU, y: std::f32::consts::TAU },
            softness: 0.0,
            intensity: gain,
            color: brush_colour(brush, notes),
            show_in_background: true,
            in_lighting: true,
            blend: PinBlend::Over,
            ..Pin::default()
        });
    }
    let children = map.object_link("pins").and_then(|n| file.objects.get(&n)).and_then(|g| g.i32s("children")).unwrap_or(&[]);
    for &child in children {
        let Some(p) = file.objects.get(&(child as u32)) else {
            notes.push(format!("the environment {} lists a pin it does not hold", o.name));
            continue;
        };
        e.pins.push(pin(file, p, notes));
    }
    if !e.pins.is_empty() {
        notes.push("HDRI-editor pins are placed by azimuth and elevation in degrees, azimuth 0 looking down -Z, and sized in degrees across; KeyShot's own units for them are not documented, so this is the best reading");
    }
    e
}

/// One HDRI-editor pin.
fn pin(file: &File, p: &Container, notes: &mut Notes) -> Pin {
    let degrees = |k: &str, default: f32| p.f32(k).unwrap_or(default).to_radians();
    let circle = p.int("shape").unwrap_or(1) == 1;
    let size = if circle {
        let d = 2.0 * degrees("radius", 15.0);
        Vec2 { x: d, y: d }
    } else {
        Vec2 { x: degrees("width", 30.0), y: degrees("height", 30.0) }
    };
    let mut out = Pin {
        name: if p.name.trim().is_empty() { "Pin".into() } else { p.name.clone() },
        shape: if circle { PinShape::Ellipse } else { PinShape::Rectangle },
        direction: pins::direction_from(degrees("azimuth", 0.0), degrees("inclination", 0.0)),
        angular_size: Vec2 { x: size.x.clamp(1e-3, std::f32::consts::TAU), y: size.y.clamp(1e-3, std::f32::consts::TAU) },
        roll: if circle { 0.0 } else { degrees("rotation", 0.0) },
        softness: p.f32("falloff").unwrap_or(0.4).clamp(0.0, 1.0),
        enabled: p.flag("enabled").unwrap_or(true),
        // KeyShot's pins are part of its environment's picture: seen behind
        // the product wherever the environment is.
        show_in_background: true,
        in_lighting: true,
        blend: PinBlend::Add,
        ..Pin::default()
    };
    if p.flag("half") == Some(true) {
        notes.push("a half pin came in whole: Tequila's pins have no half");
    }
    if p.f32("corner_radius").is_some_and(|r| r > 0.0) {
        notes.push("a rectangular pin's rounded corners came in square");
    }
    let Some(brush) = p.object_link("brush").and_then(|n| file.objects.get(&n)) else {
        out.intensity = 1.0;
        return out;
    };
    out.intensity = brush.f32("color.gain").unwrap_or(1.0).max(0.0);
    if brush.int("blend_mode").is_some_and(|m| m != 0) {
        notes.push("a pin's blend mode other than KeyShot's first was taken as adding light");
    }
    if brush.class == "lux.env.brush.gradient" {
        // The stops' encoding is not understood. Read as KeyShot's opacity
        // stops (position, opacity) while they rise, the strip fades from
        // full to nothing along its length, which is what such strips are
        // made for; its colour stays the brush's.
        let mut stops: Vec<GradientStop> = Vec::new();
        for pair in brush.pairs("gradient.alpha").unwrap_or(&[]) {
            if stops.last().is_some_and(|s| pair[0] <= s.t) || !(0.0..=1.0).contains(&pair[0]) {
                break;
            }
            stops.push(GradientStop { t: pair[0], color: Spectrum::splat(pair[1].clamp(0.0, 1.0)) });
        }
        if stops.len() < 2 {
            stops = vec![GradientStop { t: 0.0, color: Spectrum::ONE }, GradientStop { t: 1.0, color: Spectrum::ONE }];
        }
        out.kind = PinKind::Gradient { stops, radial: false };
        out.color = brush_colour(brush, notes);
        notes.push("an HDRI-editor gradient pin's stops are not understood; it came in as a fade along its length, without its own angle");
    } else {
        out.color = brush_colour(brush, notes);
    }
    out
}
