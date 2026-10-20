//! STEP (ISO 10303-21), the format every CAD package can write.
//!
//! A STEP file describes exact geometry, a boundary representation of
//! surfaces trimmed by curves, and renders nothing until it is turned into
//! triangles. OpenCASCADE does that where its helper is installed
//! ([`crate::occt`], the roadmap's M8): it reads every surface CAD packages
//! write, and its answer is laid out here exactly as truck's is. Without
//! the helper, or when it fails, the pure-Rust reader below reads the file:
//! ruststep parses the exchange file and truck meshes its solids, with
//! nothing to build and no C++ toolchain. Either reads what a part exported
//! from Fusion, Onshape or SolidWorks actually contains:
//!
//! * each solid body as a part, under its own name;
//! * the body's colour, and a face's where it overrides the body's;
//! * the length unit the file's geometry context assigns;
//! * an assembly's structure: each component where the assembly places it,
//!   and a part used several times meshed once and shared.
//!
//! Geometry comes from truck, names, colours and units from the file's
//! entities directly, which truck does not read. The two meet at the entity
//! ids: a body names its shells by id, and truck's table is keyed by them.
//!
//! One bad face does not cost the file. truck panics on some geometry it
//! cannot divide; each body is meshed whole first and, if that panics, one
//! face at a time, so what is lost is the faces that failed, and the warning
//! says how many.

use crate::raw::{RawMesh, RawNode, RawScene};
use crate::ImportError;
use tequila_core::math::{vec3, Mat4, Transform};
use tequila_core::spectrum::Spectrum;
use tequila_core::units::LengthUnit;
use tequila_materials::material_type::MaterialType;
use tequila_render_core::material::Material;
use tequila_scene::material::MaterialDefinition;
use ruststep::ast::{EntityInstance, Name, Parameter, Record};
use ruststep::tables::PlaceHolder;
use std::collections::{BTreeMap, HashMap};
use truck_meshalgo::prelude::*;
use truck_stepio::r#in::{FaceAnyHolder, ShellHolder, Table};
use truck_topology::compress::CompressedShell;

/// How finely a CAD part is divided into triangles: how far a triangle may
/// stray from the true surface, as a fraction of the body's size.
///
/// Medium, a thousandth, keeps a 50 mm part's facets under 0.05 mm, which
/// normals interpolated from the exact surface hide everywhere but the
/// silhouette. A close-up of a curved edge wants High or Ultra; a large
/// assembly seen whole, Draft. The presets are `docs/03` §6.2's. The exact geometry is kept with the project
/// (`tequila_scene::document::Sources`), so a part can be divided again at
/// another quality without its file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tessellation {
    Draft,
    #[default]
    Medium,
    High,
    Ultra,
}

impl Tessellation {
    pub const ALL: [Tessellation; 4] = [Tessellation::Draft, Tessellation::Medium, Tessellation::High, Tessellation::Ultra];

    /// The chord tolerance, as a fraction of the body's size.
    pub fn chord(self) -> f64 {
        match self {
            Tessellation::Draft => 5.0e-3,
            Tessellation::Medium => 1.0e-3,
            Tessellation::High => 5.0e-4,
            Tessellation::Ultra => 1.0e-4,
        }
    }

    /// How far a curve may turn between one triangle edge and the next, in
    /// radians: OpenCASCADE's mesher bounds the angle as well as the chord,
    /// so a small fillet is divided as smoothly as a large one.
    pub fn angle(self) -> f64 {
        match self {
            Tessellation::Draft => 0.8,
            Tessellation::Medium => 0.5,
            Tessellation::High => 0.35,
            Tessellation::Ultra => 0.2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tessellation::Draft => "Draft",
            Tessellation::Medium => "Medium",
            Tessellation::High => "High",
            Tessellation::Ultra => "Ultra",
        }
    }

    pub fn from_label(label: &str) -> Option<Tessellation> {
        Tessellation::ALL.into_iter().find(|t| t.label().eq_ignore_ascii_case(label.trim()))
    }
}

/// How far the STEP reads under way have got, summed over all of them: the
/// faces to divide and how many are done, for a status line. A big
/// assembly takes minutes, and an import that shows nothing for that long
/// reads as one that has hung. `None` when nothing is being read.
pub fn progress() -> Option<Progress> {
    let reading = READING.lock().ok()?;
    if reading.is_empty() {
        return None;
    }
    let mut sum = Progress::default();
    for one in reading.iter() {
        sum.done += one.done.load(std::sync::atomic::Ordering::Relaxed);
        sum.total += one.total.load(std::sync::atomic::Ordering::Relaxed);
        sum.read = sum.read.max(one.read.load(std::sync::atomic::Ordering::Relaxed));
    }
    Some(sum)
}

/// See [`progress`]. A total of zero means the file is still being read;
/// `read` is how far through that reading is, in percent, where the reader
/// can say (OpenCASCADE can, truck cannot).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub read: usize,
}

#[derive(Default)]
struct Counts {
    done: std::sync::atomic::AtomicUsize,
    total: std::sync::atomic::AtomicUsize,
    read: std::sync::atomic::AtomicUsize,
}

static READING: std::sync::Mutex<Vec<std::sync::Arc<Counts>>> = std::sync::Mutex::new(Vec::new());

/// One read's counts, listed while it lasts.
struct Reading(std::sync::Arc<Counts>);

impl Reading {
    fn start() -> Reading {
        let counts = std::sync::Arc::new(Counts::default());
        if let Ok(mut reading) = READING.lock() {
            reading.push(counts.clone());
        }
        Reading(counts)
    }
}

impl Drop for Reading {
    fn drop(&mut self) {
        if let Ok(mut reading) = READING.lock() {
            reading.retain(|counts| !std::sync::Arc::ptr_eq(counts, &self.0));
        }
    }
}

/// The metadata field a STEP model's top nodes record their tessellation in.
pub const TESSELLATION: &str = "Tessellation";

/// Read a STEP file at the default quality.
pub fn read(bytes: &[u8], name: &str) -> Result<RawScene, ImportError> {
    read_with(bytes, name, Tessellation::default())
}

/// Read a STEP file, dividing its surfaces at `quality`.
pub fn read_with(bytes: &[u8], name: &str, quality: Tessellation) -> Result<RawScene, ImportError> {
    let reading = Reading::start();
    // OpenCASCADE where it is installed (`crate::occt`), and truck where it
    // is not or cannot read the file.
    let mut fell_back = None;
    if let Some(helper) = crate::occt::helper() {
        let counts = reading.0.clone();
        let report = move |step: crate::occt::Step| match step {
            crate::occt::Step::Reading(percent) => counts.read.store(percent, std::sync::atomic::Ordering::Relaxed),
            crate::occt::Step::Dividing(done, total) => {
                counts.total.store(total.max(1), std::sync::atomic::Ordering::Relaxed);
                counts.done.store(done, std::sync::atomic::Ordering::Relaxed);
            }
        };
        match crate::occt::read(&helper, bytes, quality.chord(), quality.angle(), &report) {
            Ok(read) => return from_occt(read, &predefined_in(bytes), name, quality),
            Err(why) => fell_back = Some(why),
        }
        reading.0.done.store(0, std::sync::atomic::Ordering::Relaxed);
        reading.0.total.store(0, std::sync::atomic::Ordering::Relaxed);
        reading.0.read.store(0, std::sync::atomic::Ordering::Relaxed);
    }
    let text = String::from_utf8_lossy(bytes);
    let data = crate::part21::data_section(&text)
        .map_err(|e| ImportError::Malformed(format!("this is not a STEP file Tequila can read: {e}")))?;

    let entities = Entities::new(&data.entities);
    let table = Table::from_data_section(&data);
    let mut warnings = Vec::new();

    let source_unit = entities.length_unit(&mut warnings);
    let colours = entities.colours();
    let bodies = entities.bodies(&table);
    if bodies.is_empty() {
        return Err(ImportError::Unsupported(
            "the STEP file has no solids or surfaces to mesh — only drawings, \
             wireframe or product data"
                .into(),
        ));
    }
    // One tolerance scale for the whole file, so a tiny body does not get
    // a tolerance below what its model can see.
    let model_size = bodies.iter().map(|b| b.size(&table)).fold(0.0, f64::max);

    let mut scene = RawScene { source_unit, ..RawScene::default() };
    let mut materials: Vec<(Colour, usize)> = Vec::new();
    // Each body meshed once, however often an assembly uses it: its
    // instances share the meshes. Bodies are independent, so they are
    // meshed side by side, and numbered in file order afterwards so the
    // result does not depend on which finished first.
    let names: Vec<String> = bodies
        .iter()
        .enumerate()
        .map(|(index, body)| if body.name.trim().is_empty() { format!("Body {}", index + 1) } else { body.name.clone() })
        .collect();
    let groups: Vec<_> = bodies.iter().map(|body| body.groups(&table, &colours)).collect();
    let faces: usize = groups.iter().flatten().map(|(_, faces, _)| faces.len()).sum();
    reading.0.total.store(faces.max(1), std::sync::atomic::Ordering::Relaxed);
    let results: Vec<(Vec<(Option<Colour>, RawMesh)>, usize, usize)> = {
        use rayon::prelude::*;
        bodies
            .par_iter()
            .zip(groups)
            .map(|(body, groups)| {
                let tolerance = (body.size(&table) * quality.chord())
                    .max(model_size * quality.chord() * 0.1)
                    .max(10.0 * truck_meshalgo::prelude::TOLERANCE);
                let mut meshes = Vec::new();
                let (mut failed, mut total) = (0, 0);
                for (colour, faces, inverted) in groups {
                    let count = faces.len();
                    total += count;
                    let holder = ShellHolder { label: String::new(), cfs_faces: faces };
                    let (mesh, lost) = mesh_faces(&table, &holder, tolerance, inverted);
                    failed += lost;
                    meshes.extend(mesh.map(|mesh| (colour, mesh)));
                    reading.0.done.fetch_add(count, std::sync::atomic::Ordering::Relaxed);
                }
                (meshes, failed, total)
            })
            .collect()
    };
    let mut meshed: Vec<Vec<(Option<Colour>, usize)>> = Vec::with_capacity(bodies.len());
    for ((meshes, failed, total), body_name) in results.into_iter().zip(&names) {
        if failed > 0 {
            warnings.push(format!("{body_name}: {failed} of {total} faces could not be meshed and are missing"));
        }
        let mut entries = Vec::new();
        for (colour, mut mesh) in meshes {
            mesh.material = material_of(colour, &mut scene.materials, &mut materials);
            mesh.name = body_name.clone();
            scene.meshes.push(mesh);
            entries.push((colour, scene.meshes.len() - 1));
        }
        meshed.push(entries);
    }
    if meshed.iter().all(Vec::is_empty) {
        return Err(ImportError::Malformed(format!(
            "no face in the STEP file could be meshed{}",
            warnings.first().map(|w| format!(" ({w})")).unwrap_or_default()
        )));
    }

    let fallback = entities.product_name().unwrap_or_else(|| name.to_string());
    let products = entities.structure(&bodies, &fallback, &mut warnings);
    let mut layout = Layout { scene: &mut scene, meshed: &meshed, names: &names };
    match products.as_slice() {
        [product] => layout.product(product, None),
        several => {
            let group = layout.push(RawNode::group(name), None, Transform::IDENTITY);
            for product in several {
                layout.product(product, Some(group));
            }
        }
    }
    if let Some(why) = fell_back {
        warnings.insert(0, format!("{why}; read with Tequila's own reader instead"));
    }
    scene.warnings = warnings;
    scene.tessellation = Some(quality.label().to_string());
    Ok(scene)
}

/// What OpenCASCADE read, laid out as truck's reading is: the same bodies,
/// colours, products and names, so a file reads the same either way but
/// for the faces truck lost.
fn from_occt(read: crate::occt::Read, named: &[Colour], name: &str, quality: Tessellation) -> Result<RawScene, ImportError> {
    let near = |mm: f64| (read.unit_mm - mm).abs() < 1e-6 * mm;
    let source_unit = if near(1.0) {
        Some(LengthUnit::Millimetre)
    } else if near(10.0) {
        Some(LengthUnit::Centimetre)
    } else if near(1000.0) {
        Some(LengthUnit::Metre)
    } else if near(25.4) {
        Some(LengthUnit::Inch)
    } else if near(304.8) {
        Some(LengthUnit::Foot)
    } else {
        None
    };
    let mut scene = RawScene { source_unit, ..RawScene::default() };
    let mut warnings = Vec::new();
    if source_unit.is_none() {
        warnings.push("the STEP file's length unit is not one Tequila knows; its size is guessed".into());
    }
    let mut materials: Vec<(Colour, usize)> = Vec::new();
    let mut meshed: Vec<Vec<(Option<Colour>, usize)>> = Vec::with_capacity(read.bodies.len());
    let mut names = Vec::with_capacity(read.bodies.len());
    for (index, body) in read.bodies.into_iter().enumerate() {
        let body_name = if body.name.trim().is_empty() { format!("Body {}", index + 1) } else { body.name };
        if body.lost > 0 {
            warnings.push(format!("{body_name}: {} of {} faces could not be meshed and are missing", body.lost, body.faces));
        }
        let mut entries = Vec::new();
        for (colour, mut mesh) in body.groups {
            if mesh.indices.is_empty() {
                continue;
            }
            let colour = colour.map(|rgb| named.iter().find(|c| c.rgb().iter().zip(rgb).all(|(a, b)| (a - b).abs() < 1e-3)).copied().unwrap_or(Colour::Rgb(rgb)));
            mesh.material = material_of(colour, &mut scene.materials, &mut materials);
            mesh.name = body_name.clone();
            scene.meshes.push(mesh);
            entries.push((colour, scene.meshes.len() - 1));
        }
        meshed.push(entries);
        names.push(body_name);
    }
    if scene.meshes.is_empty() {
        return Err(ImportError::Unsupported(
            "the STEP file has no solids or surfaces to mesh — only drawings, \
             wireframe or product data"
                .into(),
        ));
    }
    fn product(p: crate::occt::Product) -> Product {
        let mut m = Mat4::IDENTITY;
        for row in 0..4 {
            for col in 0..4 {
                m.m[row][col] = p.matrix[row * 4 + col];
            }
        }
        Product {
            name: if p.name.trim().is_empty() { "Part".into() } else { p.name },
            transform: Transform::from_matrix(m).unwrap_or(Transform::IDENTITY),
            bodies: p.bodies,
            children: p.children.into_iter().map(product).collect(),
        }
    }
    let products: Vec<Product> = read.products.into_iter().map(product).collect();
    let mut layout = Layout { scene: &mut scene, meshed: &meshed, names: &names };
    match products.as_slice() {
        [product] => layout.product(product, None),
        several => {
            let group = layout.push(RawNode::group(name), None, Transform::IDENTITY);
            for product in several {
                layout.product(product, Some(group));
            }
        }
    }
    scene.warnings = warnings;
    scene.tessellation = Some(quality.label().to_string());
    Ok(scene)
}

/// The material for a colour, made the first time the colour is seen.
fn material_of(
    colour: Option<Colour>,
    definitions: &mut Vec<MaterialDefinition>,
    seen: &mut Vec<(Colour, usize)>,
) -> Option<usize> {
    let colour = colour?;
    if let Some((_, index)) = seen.iter().find(|(c, _)| *c == colour) {
        return Some(*index);
    }
    let rgb = colour.rgb();
    let material = Material::plastic(Spectrum::from_srgb_encoded(vec3(rgb[0], rgb[1], rgb[2])), 0.2);
    definitions.push(MaterialDefinition::new(colour.name(), MaterialType::Plastic, material));
    seen.push((colour, definitions.len() - 1));
    Some(definitions.len() - 1)
}

/// A product as the file structures it: its own bodies, and the components
/// it places.
struct Product {
    name: String,
    /// Where the product sits in its parent; identity for a root.
    transform: Transform,
    /// Indices into the file's bodies.
    bodies: Vec<usize>,
    children: Vec<Product>,
}

/// Turns products into nodes over the meshes already made.
struct Layout<'a> {
    scene: &'a mut RawScene,
    /// Each body's meshes, and the colour each renders in.
    meshed: &'a [Vec<(Option<Colour>, usize)>],
    names: &'a [String],
}

impl Layout<'_> {
    fn push(&mut self, mut node: RawNode, parent: Option<usize>, transform: Transform) -> usize {
        node.parent = parent;
        node.transform = transform;
        self.scene.nodes.push(node);
        let index = self.scene.nodes.len() - 1;
        if parent.is_none() {
            self.scene.roots.push(index);
        }
        index
    }

    /// A product is a group of its bodies and components. One with a
    /// single body and nothing else is that body, under the product's name:
    /// Fusion calls every body "Body1", and the product is what the user
    /// named.
    fn product(&mut self, product: &Product, parent: Option<usize>) {
        let bodies: Vec<usize> = product.bodies.iter().copied().filter(|&b| !self.meshed[b].is_empty()).collect();
        if product.children.is_empty() {
            match bodies.as_slice() {
                [] => return,
                [body] => return self.body(*body, &product.name, parent, product.transform),
                _ => {}
            }
        }
        let group = self.push(RawNode::group(product.name.clone()), parent, product.transform);
        for body in bodies {
            let name = self.names[body].clone();
            self.body(body, &name, Some(group), Transform::IDENTITY);
        }
        for child in &product.children {
            self.product(child, Some(group));
        }
    }

    /// A body is a part, or, where its faces carry colours of their own,
    /// a group of one part per colour, since a part has one material.
    fn body(&mut self, body: usize, name: &str, parent: Option<usize>, transform: Transform) {
        match self.meshed[body].as_slice() {
            [] => {}
            [(_, mesh)] => {
                self.push(RawNode::part(name, *mesh), parent, transform);
            }
            several => {
                let group = self.push(RawNode::group(name), parent, transform);
                for &(colour, mesh) in several {
                    let part = match colour {
                        Some(colour) => format!("{name} ({})", colour.name()),
                        None => name.to_string(),
                    };
                    self.push(RawNode::part(part, mesh), Some(group), Transform::IDENTITY);
                }
            }
        }
    }
}

/// Mesh some faces of a shell: all at once, or one at a time if that
/// panics. Returns the mesh and how many faces were lost.
///
/// Meshed in truck's compressed form, never extracted into a topological
/// shell: extraction refuses an edge that starts and ends at one vertex,
/// and a full circle drawn with a single vertex is how STEP writes most
/// holes and shafts.
fn mesh_faces(table: &Table, holder: &ShellHolder, tolerance: f64, inverted: bool) -> (Option<RawMesh>, usize) {
    let wanted = holder.cfs_faces.len();
    let Ok(mut compressed) = table.to_compressed_shell(holder) else {
        return (None, wanted);
    };
    // truck drops a face whose surface it cannot read rather than failing
    // the shell, so what did not arrive was lost here.
    let mut lost = wanted.saturating_sub(compressed.faces.len());
    normalize_knots(&mut compressed);
    if inverted {
        for face in &mut compressed.faces {
            face.orientation = !face.orientation;
        }
    }
    let whole = tequila_core::guard::catching(|| compressed.robust_triangulation(tolerance));
    // Each divided shell, and for one face divided alone, which face.
    let meshed: Vec<(Option<usize>, CompressedShell<Point3, PolylineCurve<Point3>, Option<PolygonMesh>>)> = match whole {
        Ok(meshed) => vec![(None, meshed)],
        // One face truck cannot divide fails the whole shell, so each face
        // is divided alone and only the ones that fail are lost - each with
        // just its own edges, side by side.
        Err(_) => {
            use rayon::prelude::*;
            (0..compressed.faces.len())
                .into_par_iter()
                .filter_map(|i| {
                    let one = face_alone(&compressed, i)?;
                    tequila_core::guard::catching(|| one.robust_triangulation(tolerance)).ok().map(|m| (Some(i), m))
                })
                .collect()
        }
    };
    // Each face's division by its index, settled against its own boundary.
    let mut divided: Vec<Option<PolygonMesh>> = vec![None; compressed.faces.len()];
    for (alone, shell) in &meshed {
        for (f, face) in shell.faces.iter().enumerate() {
            if let Some(slot) = divided.get_mut(alone.unwrap_or(f)) {
                *slot = face.surface.clone();
            }
        }
    }
    let divided: Vec<Option<PolygonMesh>> = {
        use rayon::prelude::*;
        divided.into_par_iter().enumerate().map(|(index, polygon)| settle(&compressed, index, polygon, tolerance)).collect()
    };
    // The B-rep's edges, as the triangulation divided them, so they meet
    // the triangles exactly. A seam - an edge a cylinder or a sphere meets
    // itself along - is left out: it is a place in the parameterization,
    // not an edge of the part, and CAD packages do not draw it either.
    let mut users: Vec<Vec<usize>> = vec![Vec::new(); compressed.edges.len()];
    for (f, face) in compressed.faces.iter().enumerate() {
        for wire in &face.boundaries {
            for edge in wire {
                if let Some(list) = users.get_mut(edge.index) {
                    list.push(f);
                }
            }
        }
    }
    let seam = |e: usize| users[e].len() >= 2 && users[e].iter().all(|&f| f == users[e][0]);
    let mut drawn = vec![false; compressed.edges.len()];
    let mut edge_lines = Vec::new();
    for (_, shell) in &meshed {
        for face in shell.faces.iter().filter(|f| f.surface.is_some()) {
            for wire in &face.boundaries {
                for edge in wire {
                    if edge.index >= drawn.len() || drawn[edge.index] || seam(edge.index) {
                        continue;
                    }
                    drawn[edge.index] = true;
                    let Some(curve) = shell.edges.get(edge.index) else { continue };
                    for pair in curve.curve.0.windows(2) {
                        for p in pair {
                            edge_lines.push(vec3(p.x as f32, p.y as f32, p.z as f32));
                        }
                    }
                }
            }
        }
    }

    let mut polygons = PolygonMesh::default();
    let mut meshed_faces = 0;
    for (index, polygon) in divided.into_iter().enumerate() {
        let Some(mut polygon) = polygon else { continue };
        meshed_faces += 1;
        // As truck's own shells merge: a face facing against its surface
        // turned round.
        if !compressed.faces[index].orientation {
            polygon.invert();
        }
        polygons.merge(polygon);
    }
    lost += compressed.faces.len().saturating_sub(meshed_faces);
    let mut mesh = raw_mesh(&polygons);
    mesh.edge_lines = edge_lines;
    ((!mesh.indices.is_empty()).then_some(mesh), lost)
}

/// Every B-spline and NURBS surface's knots rescaled to run 0 to 1, which
/// leaves the surface exactly where it was.
///
/// truck's tessellation works in a surface's parameters with tolerances
/// fixed for a domain about 1 wide: a boundary counts as closed within
/// 1e-3, a parameter matches within 1e-6. A CAD package that writes a
/// fillet's rational surface over u in 0.0055..0.0064 makes the whole
/// domain narrower than "closed", so finding the face's boundary on its
/// surface went astray - points at u -0.05, v -2.3 - and the division
/// that followed ran out along the surface extended far past its edge,
/// splitting without end: one face of a 57 MB assembly took every core and
/// grew past 9 GB, and the import never finished. Rescaled, the same faces
/// divide at once. (Measured on the face itself, alone.)
///
/// A surface whose knots span next to nothing is left as it is: truck's
/// normalizing panics on it, and there is no width to rescale.
fn normalize_knots<P, C>(shell: &mut CompressedShell<P, C, truck_stepio::r#in::alias::Surface>) {
    use truck_stepio::r#in::alias::Surface;
    let wide = |u: f64, v: f64| u > 1e-5 && v > 1e-5;
    for face in &mut shell.faces {
        let _ = tequila_core::guard::catching(|| match &mut face.surface {
            Surface::BSplineSurface(surface) if wide(surface.uknot_vec().range_length(), surface.vknot_vec().range_length()) => {
                surface.knot_normalize();
            }
            Surface::NurbsSurface(surface) if wide(surface.uknot_vec().range_length(), surface.vknot_vec().range_length()) => {
                surface.knot_normalize();
            }
            _ => {}
        });
    }
}

fn span<'a>(points: impl Iterator<Item = &'a Point3>) -> Option<([f64; 3], [f64; 3])> {
    let mut found = None::<([f64; 3], [f64; 3])>;
    for p in points {
        let (lo, hi) = found.get_or_insert(([p.x, p.y, p.z], [p.x, p.y, p.z]));
        for (a, v) in [p.x, p.y, p.z].into_iter().enumerate() {
            lo[a] = lo[a].min(v);
            hi[a] = hi[a].max(v);
        }
    }
    found
}

fn extent((lo, hi): ([f64; 3], [f64; 3])) -> f64 {
    (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f64::max)
}

type StepShell = CompressedShell<Point3, truck_stepio::r#in::alias::Curve3D, truck_stepio::r#in::alias::Surface>;

/// Face `index`'s boundary loops in 3D: its edges divided at `tolerance`,
/// each the way its wire runs. `None` if an edge is missing or will not
/// divide.
fn outline(compressed: &StepShell, index: usize, tolerance: f64) -> Option<Vec<Vec<Point3>>> {
    let face = compressed.faces.get(index)?;
    face.boundaries
        .iter()
        .map(|wire| {
            let mut points: Vec<Point3> = Vec::new();
            for used in wire {
                let curve = &compressed.edges.get(used.index)?.curve;
                let mut line = tequila_core::guard::catching(|| PolylineCurve::from_curve(curve, curve.range_tuple(), tolerance).0).ok()?;
                if !used.orientation {
                    line.reverse();
                }
                // Each edge starts where the last ended.
                let skip = usize::from(!points.is_empty());
                points.extend(line.into_iter().skip(skip));
            }
            Some(points)
        })
        .collect()
}

/// Face `index`'s division, settled against its own boundary.
///
/// Cylinders, cones and planes come from truck-stepio with a parameter
/// domain a unit wide - a metre, in a file in metres - and where truck
/// takes a face's outer boundary for a hole (it ran the wrong way round in
/// the surface's parameters) it divides the whole domain around it; where
/// the boundary crosses a cylinder's seam and does not close there, it
/// closes it along the domain's edge; and in a file in metres a face's
/// boundary can come back from truck's parameter search somewhere else on
/// the surface entirely. A 5 mm screw thread came out a metre long, and a
/// 57 MB assembly as a burst of black sheets.
///
/// So a division that is no bigger than its boundary allows (a dome on a
/// circular base is as wide as its base) is kept. Otherwise, or with none,
/// a plane is triangulated from its outline directly, needing nothing of
/// truck's parameters, and any other surface's division is cut to its
/// boundary's box - every triangle with a corner outside goes. truck is
/// not asked again: dividing such a face a second time, finer or with its
/// boundary reversed, recovered many faces in testing but hung on a dozen
/// cylinders and spheres of the same file, and a hung thread cannot be
/// stopped. What none of this saves is lost, and counted.
fn settle(compressed: &StepShell, index: usize, polygon: Option<PolygonMesh>, tolerance: f64) -> Option<PolygonMesh> {
    let loops = outline(compressed, index, tolerance);
    // Failing the outline, the box around its edges' ends - with room for
    // an arc's bulge past them.
    let ends = || {
        let face = compressed.faces.get(index)?;
        let ends: Vec<Point3> = face
            .boundaries
            .iter()
            .flatten()
            .filter_map(|e| compressed.edges.get(e.index))
            .flat_map(|e| [e.vertices.0, e.vertices.1])
            .filter_map(|v| compressed.vertices.get(v))
            .collect();
        let (lo, hi) = span(ends.iter())?;
        let grow = 0.5 * extent((lo, hi));
        Some((lo.map(|v| v - grow), hi.map(|v| v + grow)))
    };
    let Some(boundary) = loops.as_ref().and_then(|l| span(l.iter().flatten())).or_else(ends) else { return polygon };
    let width = extent(boundary);
    let fits = |m: &PolygonMesh| m.faces().len() > 0 && span(m.positions().iter()).is_some_and(|b| extent(b) <= 1.5 * width + 10.0 * tolerance);
    if let Some(polygon) = &polygon {
        if fits(polygon) {
            return Some(polygon.clone());
        }
    }
    if let Some(flat) = loops.as_ref().and_then(|l| flat_face(compressed, index, l)) {
        return Some(flat);
    }
    let polygon = polygon?;
    let margin = 0.05 * width + 2.0 * tolerance;
    let (lo, hi) = boundary;
    let inside = |p: Point3| [p.x, p.y, p.z].into_iter().enumerate().all(|(a, v)| v >= lo[a] - margin && v <= hi[a] + margin);
    let positions = polygon.positions();
    let kept: Vec<[StandardVertex; 3]> = polygon
        .faces()
        .triangle_iter()
        .filter(|t| t.iter().all(|v| inside(positions[v.pos])))
        .collect();
    if kept.is_empty() {
        return None;
    }
    let attributes = StandardAttributes {
        positions: positions.to_vec(),
        uv_coords: polygon.uv_coords().to_vec(),
        normals: polygon.normals().to_vec(),
    };
    Some(PolygonMesh::debug_new(attributes, Faces::from_tri_and_quad_faces(kept, Vec::new())))
}

/// A plane's face triangulated from its outline: the loops laid flat in
/// the plane and filled by the even-odd rule (`crate::planar`), which
/// needs no telling which loop is outer or which way any runs - just what
/// truck got wrong. Facing the plane's own normal, as truck's divisions
/// do before a face's orientation is applied.
fn flat_face(compressed: &StepShell, index: usize, loops: &[Vec<Point3>]) -> Option<PolygonMesh> {
    use truck_stepio::r#in::alias::{ElementarySurface, Surface};
    let Surface::ElementarySurface(surface) = &compressed.faces.get(index)?.surface else { return None };
    let ElementarySurface::Plane(plane) = surface.as_ref() else { return None };
    let origin = plane.subs(0.0, 0.0);
    let normal = plane.normal();
    let across = plane.u_axis().normalize();
    let up = normal.cross(across);
    let flat: Vec<Vec<crate::planar::P2>> = loops
        .iter()
        .map(|l| l.iter().map(|p| [(p - origin).dot(across), (p - origin).dot(up)]).collect())
        .collect();
    let (points, triangles) = crate::planar::triangulate(&flat, &[]);
    if triangles.is_empty() {
        return None;
    }
    let attributes = StandardAttributes {
        positions: points.iter().map(|q| origin + across * q[0] + up * q[1]).collect(),
        uv_coords: points.iter().map(|q| Vector2::new(q[0], q[1])).collect(),
        normals: vec![normal],
    };
    let vertex = |i: usize| StandardVertex { pos: i, uv: Some(i), nor: Some(0) };
    let faces: Vec<[StandardVertex; 3]> = triangles.iter().map(|t| [vertex(t[0]), vertex(t[1]), vertex(t[2])]).collect();
    Some(PolygonMesh::debug_new(attributes, Faces::from_tri_and_quad_faces(faces, Vec::new())))
}

/// truck's polygon mesh, indexed separately per attribute, as one index
/// list over vertices that carry their normal.
/// Face `i` of `shell` as a shell of its own: just the edges its boundaries
/// use and their end points, renumbered.
///
/// What the face-at-a-time retry divides. It cloned the whole shell's
/// vertices and edges - curves and all - for every face, which is the
/// shell's size times its number of faces: 20 seconds for the 676 faces of a
/// wrench that divides whole in one, and hours for a CAD part of twenty
/// thousand faces - an import that never finished.
///
/// `None` for a face that names an edge or a vertex the shell does not have:
/// truck leaves out an edge it could not read and keeps the faces that use
/// it, and taking that face alone panicked outside every guard - the whole
/// import stopped, over one face it would only have lost.
fn face_alone<P: Clone, C: Clone, S: Clone>(shell: &CompressedShell<P, C, S>, i: usize) -> Option<CompressedShell<P, C, S>> {
    use truck_topology::compress::{CompressedEdge, CompressedEdgeIndex, CompressedFace};
    let face = shell.faces.get(i)?;
    let whole = face.boundaries.iter().flatten().all(|used| {
        shell.edges.get(used.index).is_some_and(|edge| edge.vertices.0 < shell.vertices.len() && edge.vertices.1 < shell.vertices.len())
    });
    if !whole {
        return None;
    }
    let mut edges_of: HashMap<usize, usize> = HashMap::new();
    let mut vertices_of: HashMap<usize, usize> = HashMap::new();
    let (mut vertices, mut edges) = (Vec::new(), Vec::new());
    let boundaries = face
        .boundaries
        .iter()
        .map(|wire| {
            wire.iter()
                .map(|used| {
                    let index = *edges_of.entry(used.index).or_insert_with(|| {
                        let edge = &shell.edges[used.index];
                        let mut point = |v: usize| {
                            *vertices_of.entry(v).or_insert_with(|| {
                                vertices.push(shell.vertices[v].clone());
                                vertices.len() - 1
                            })
                        };
                        let ends = (point(edge.vertices.0), point(edge.vertices.1));
                        edges.push(CompressedEdge { vertices: ends, curve: edge.curve.clone() });
                        edges.len() - 1
                    });
                    CompressedEdgeIndex { index, orientation: used.orientation }
                })
                .collect()
        })
        .collect();
    Some(CompressedShell {
        vertices,
        edges,
        faces: vec![CompressedFace { boundaries, orientation: face.orientation, surface: face.surface.clone() }],
    })
}

fn raw_mesh(polygons: &PolygonMesh) -> RawMesh {
    let positions = polygons.positions();
    let normals = polygons.normals();
    let mut mesh = RawMesh::default();
    let mut vertices: HashMap<(usize, Option<usize>), u32> = HashMap::new();
    let mut every_normal = true;
    for triangle in polygons.faces().triangle_iter() {
        for vertex in triangle {
            let key = (vertex.pos, vertex.nor);
            let index = *vertices.entry(key).or_insert_with(|| {
                let p = positions[vertex.pos];
                mesh.positions.push(vec3(p.x as f32, p.y as f32, p.z as f32));
                match vertex.nor.and_then(|n| normals.get(n)) {
                    Some(n) => mesh.normals.push(vec3(n.x as f32, n.y as f32, n.z as f32)),
                    None => every_normal = false,
                }
                (mesh.positions.len() - 1) as u32
            });
            mesh.indices.push(index);
        }
    }
    // A normal per vertex or none: the pipeline regenerates an absent set,
    // and never a partial one.
    if !every_normal || mesh.normals.len() != mesh.positions.len() {
        mesh.normals.clear();
    }
    mesh
}

/// A colour a STEP file gives a body or face.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Colour {
    /// `COLOUR_RGB`, as the CAD package's picker showed it: sRGB-encoded.
    Rgb([f32; 3]),
    /// `DRAUGHTING_PRE_DEFINED_COLOUR`, one of eight named colours.
    Named(&'static str, [f32; 3]),
}

impl Colour {
    fn rgb(self) -> [f32; 3] {
        match self {
            Colour::Rgb(rgb) | Colour::Named(_, rgb) => rgb,
        }
    }

    fn name(self) -> String {
        match self {
            Colour::Named(name, _) => name.to_string(),
            Colour::Rgb(rgb) => {
                let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
                format!("Colour #{:02X}{:02X}{:02X}", byte(rgb[0]), byte(rgb[1]), byte(rgb[2]))
            }
        }
    }
}

/// A solid or surface body: its name and the shells that bound it.
struct Body {
    name: String,
    /// The shells, and whether each is a void (an inner boundary, whose
    /// faces point inward).
    shells: Vec<(u64, bool)>,
    /// The body's own entity, which a style can name.
    id: u64,
}

impl Body {
    /// The diagonal of the box around the body's vertices, in file units.
    fn size(&self, table: &Table) -> f64 {
        let mut low = [f64::INFINITY; 3];
        let mut high = [f64::NEG_INFINITY; 3];
        for (shell, _) in &self.shells {
            let Some(holder) = table.shell.get(shell) else { continue };
            let Ok(compressed) = tequila_core::guard::catching(|| table.to_compressed_shell(holder)) else { continue };
            let Ok(compressed) = compressed else { continue };
            for p in &compressed.vertices {
                for (axis, value) in [p.x, p.y, p.z].into_iter().enumerate() {
                    low[axis] = low[axis].min(value);
                    high[axis] = high[axis].max(value);
                }
            }
        }
        let d: f64 = (0..3).map(|a| (high[a] - low[a]).powi(2)).sum::<f64>().sqrt();
        if d.is_finite() { d } else { 0.0 }
    }

    /// The body's faces, split by the colour each renders in: the face's
    /// own, else its shell's, else the body's.
    ///
    /// Each group is faces of one shell, and whether that shell is a void.
    fn groups(
        &self,
        table: &Table,
        colours: &HashMap<u64, Colour>,
    ) -> Vec<(Option<Colour>, Vec<PlaceHolder<FaceAnyHolder>>, bool)> {
        let mut groups: Vec<(Option<Colour>, Vec<PlaceHolder<FaceAnyHolder>>, bool)> = Vec::new();
        for &(shell, inverted) in &self.shells {
            let Some(holder) = table.shell.get(&shell) else { continue };
            let inherited = colours.get(&shell).or_else(|| colours.get(&self.id)).copied();
            for face in &holder.cfs_faces {
                let own = match face {
                    PlaceHolder::Ref(Name::Entity(id)) => colours.get(id).copied(),
                    _ => None,
                };
                let colour = own.or(inherited);
                match groups.iter_mut().find(|(c, _, v)| *c == colour && *v == inverted) {
                    Some((_, faces, _)) => faces.push(face.clone()),
                    None => groups.push((colour, vec![face.clone()], inverted)),
                }
            }
        }
        groups
    }
}

/// Every entity instance in the file by id, simple or complex.
pub(crate) struct Entities<'a> {
    by_id: BTreeMap<u64, Vec<&'a Record>>,
}

impl<'a> Entities<'a> {
    pub(crate) fn new(instances: &'a [EntityInstance]) -> Self {
        let mut by_id = BTreeMap::new();
        for instance in instances {
            match instance {
                EntityInstance::Simple { id, record } => {
                    by_id.insert(*id, vec![record]);
                }
                EntityInstance::Complex { id, subsuper } => {
                    by_id.insert(*id, subsuper.0.iter().collect());
                }
            }
        }
        Entities { by_id }
    }

    /// The first record of `id`, whatever it is.
    pub(crate) fn first(&self, id: u64) -> Option<&'a Record> {
        self.by_id.get(&id)?.first().copied()
    }

    /// Every instance's id, in order.
    pub(crate) fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.by_id.keys().copied()
    }

    /// The record of `id` with the given entity name, where the instance
    /// is, or includes, one.
    pub(crate) fn record(&self, id: u64, name: &str) -> Option<&'a Record> {
        self.by_id.get(&id)?.iter().copied().find(|r| r.name == name)
    }

    fn is(&self, id: u64, name: &str) -> bool {
        self.record(id, name).is_some()
    }

    /// Every simple or complex instance with a record of this name, in
    /// id order.
    pub(crate) fn named<'s>(&'s self, name: &'s str) -> impl Iterator<Item = (u64, &'a Record)> + 's {
        self.by_id
            .iter()
            .filter_map(move |(id, records)| records.iter().find(|r| r.name == name).map(|r| (*id, *r)))
    }

    /// The first product's name, which a CAD package sets to the part or
    /// assembly's.
    fn product_name(&self) -> Option<String> {
        let (_, product) = self.named("PRODUCT").next()?;
        let args = arguments(product);
        [1, 0]
            .into_iter()
            .filter_map(|i| args.get(i).and_then(string))
            .find(|s| !s.trim().is_empty())
    }

    /// The length unit the geometry context assigns.
    ///
    /// The file may define other length units — an inch is defined as 25.4
    /// of a millimetre, which is itself a length unit — so the one that
    /// counts is the one `GLOBAL_UNIT_ASSIGNED_CONTEXT` lists.
    fn length_unit(&self, warnings: &mut Vec<String>) -> Option<LengthUnit> {
        let mut found: Option<LengthUnit> = None;
        for (_, context) in self.named("GLOBAL_UNIT_ASSIGNED_CONTEXT") {
            let Some(units) = arguments(context).first().map(references) else { continue };
            for unit in units {
                if !self.is(unit, "LENGTH_UNIT") {
                    continue;
                }
                let this = self.unit(unit);
                match (this, found) {
                    (None, _) => warnings.push(
                        "the STEP file's length unit is not one Tequila knows; its size is guessed".into(),
                    ),
                    (Some(this), None) => found = Some(this),
                    (Some(this), Some(first)) if this != first => warnings.push(format!(
                        "the STEP file uses more than one length unit; {} is assumed throughout",
                        first.symbol()
                    )),
                    _ => {}
                }
            }
        }
        found
    }

    fn unit(&self, id: u64) -> Option<LengthUnit> {
        if let Some(si) = self.record(id, "SI_UNIT") {
            let args = arguments(si);
            let prefix = match args.first() {
                Some(Parameter::Enumeration(prefix)) => Some(prefix.as_str()),
                _ => None,
            };
            return match (prefix, args.get(1)) {
                (_, Some(Parameter::Enumeration(name))) if name != "METRE" => None,
                (None, _) => Some(LengthUnit::Metre),
                (Some("MILLI"), _) => Some(LengthUnit::Millimetre),
                (Some("CENTI"), _) => Some(LengthUnit::Centimetre),
                _ => None,
            };
        }
        let converted = self.record(id, "CONVERSION_BASED_UNIT")?;
        match arguments(converted).first().and_then(string)?.trim().to_ascii_uppercase().as_str() {
            "INCH" | "IN" => Some(LengthUnit::Inch),
            "FOOT" | "FT" => Some(LengthUnit::Foot),
            _ => None,
        }
    }

    /// Every body in the file, and every shell no body claims as its own.
    fn bodies(&self, table: &Table) -> Vec<Body> {
        let mut bodies = Vec::new();
        for (id, records) in &self.by_id {
            for record in records {
                let args = arguments(record);
                let name = args.first().and_then(string).unwrap_or_default();
                let shells: Vec<(u64, bool)> = match record.name.as_str() {
                    "MANIFOLD_SOLID_BREP" | "FACETED_BREP" => {
                        args.get(1).and_then(reference).map(|s| vec![(s, false)]).unwrap_or_default()
                    }
                    "BREP_WITH_VOIDS" => {
                        let mut shells: Vec<(u64, bool)> =
                            args.get(1).and_then(reference).map(|s| vec![(s, false)]).unwrap_or_default();
                        for void in args.get(2).map(references).unwrap_or_default() {
                            if let Some(shell) = self.oriented_shell(void) {
                                shells.push(shell);
                            }
                        }
                        shells
                    }
                    "SHELL_BASED_SURFACE_MODEL" => args
                        .get(1)
                        .map(references)
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|s| self.oriented_shell(s).or(Some((s, false))))
                        .collect(),
                    _ => continue,
                };
                let shells: Vec<(u64, bool)> = shells.into_iter().filter(|(s, _)| table.shell.contains_key(s)).collect();
                if !shells.is_empty() {
                    bodies.push(Body { name, shells, id: *id });
                }
            }
        }
        let claimed: std::collections::HashSet<u64> =
            bodies.iter().flat_map(|b| b.shells.iter().map(|(s, _)| *s)).collect();
        let mut loose: Vec<u64> = table.shell.keys().copied().filter(|s| !claimed.contains(s)).collect();
        loose.sort_unstable();
        for shell in loose {
            let name = table.shell[&shell].label.clone();
            bodies.push(Body { name, shells: vec![(shell, false)], id: shell });
        }
        bodies
    }

    /// The product structure: every product no other product uses, with
    /// its bodies and, through `NEXT_ASSEMBLY_USAGE_OCCURRENCE`, the
    /// components it places, each where its
    /// `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION` puts it. Bodies no product
    /// reaches belong to the first root, or, with no products at all, to
    /// one named `fallback`.
    fn structure(&self, bodies: &[Body], fallback: &str, warnings: &mut Vec<String>) -> Vec<Product> {
        let body_of: HashMap<u64, usize> = bodies.iter().enumerate().map(|(i, b)| (b.id, i)).collect();

        // Representations joined without a transformation describe one
        // shape: a part's placement in one, its B-rep in the other.
        let mut linked: HashMap<u64, Vec<u64>> = HashMap::new();
        for (id, relation) in self.named("SHAPE_REPRESENTATION_RELATIONSHIP") {
            if self.is(id, "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION") {
                continue;
            }
            let args = arguments(relation);
            if let (Some(a), Some(b)) = (args.get(2).and_then(reference), args.get(3).and_then(reference)) {
                linked.entry(a).or_default().push(b);
                linked.entry(b).or_default().push(a);
            }
        }
        let mut shapes: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        for (_, definition) in self.named("SHAPE_DEFINITION_REPRESENTATION") {
            let args = arguments(definition);
            let (Some(shape), Some(representation)) = (args.first().and_then(reference), args.get(1).and_then(reference)) else {
                continue;
            };
            let Some(pd) = self.shape_definition(shape) else { continue };
            if self.is(pd, "PRODUCT_DEFINITION") {
                shapes.entry(pd).or_default().push(representation);
            }
        }
        // Each product's representations, followed through the links.
        let representations: BTreeMap<u64, Vec<u64>> = shapes
            .iter()
            .map(|(&pd, starts)| {
                let mut seen: Vec<u64> = Vec::new();
                let mut stack = starts.clone();
                while let Some(r) = stack.pop() {
                    if !seen.contains(&r) {
                        seen.push(r);
                        stack.extend(linked.get(&r).into_iter().flatten().copied());
                    }
                }
                (pd, seen)
            })
            .collect();
        let bodies_of = |pd: u64| -> Vec<usize> {
            let mut found = Vec::new();
            for &r in representations.get(&pd).into_iter().flatten() {
                let Some(records) = self.by_id.get(&r) else { continue };
                for record in records {
                    for item in arguments(record).get(1).map(references).unwrap_or_default() {
                        if let Some(&b) = body_of.get(&item) {
                            if !found.contains(&b) {
                                found.push(b);
                            }
                        }
                    }
                }
            }
            found
        };

        // Where each usage places its component.
        let mut placements: HashMap<u64, (u64, u64, Transform)> = HashMap::new();
        for (_, dependent) in self.named("CONTEXT_DEPENDENT_SHAPE_REPRESENTATION") {
            let args = arguments(dependent);
            let (Some(relation), Some(shape)) = (args.first().and_then(reference), args.get(1).and_then(reference)) else {
                continue;
            };
            let Some(usage) = self.shape_definition(shape) else { continue };
            if let Some(placement) = self.placement_of(relation) {
                placements.insert(usage, placement);
            }
        }
        let mut children: BTreeMap<u64, Vec<(u64, u64)>> = BTreeMap::new();
        let mut used = std::collections::HashSet::new();
        let mut unplaced = 0;
        for (usage, occurrence) in self.named("NEXT_ASSEMBLY_USAGE_OCCURRENCE") {
            let args = arguments(occurrence);
            let (Some(parent), Some(child)) = (args.get(3).and_then(reference), args.get(4).and_then(reference)) else {
                continue;
            };
            children.entry(parent).or_default().push((usage, child));
            used.insert(child);
            if !placements.contains_key(&usage) {
                unplaced += 1;
            }
        }
        if unplaced > 0 {
            warnings.push(format!(
                "{unplaced} assembly component{} placed in a way Tequila does not read yet, and \
                 appear{} where modelled",
                if unplaced == 1 { " is" } else { "s are" },
                if unplaced == 1 { "s" } else { "" },
            ));
        }

        let transform_for = |usage: u64, child: u64| -> Transform {
            let Some((first, second, transform)) = placements.get(&usage) else {
                return Transform::IDENTITY;
            };
            // The relationship maps its first representation into its
            // second, and the standard does not say which is the
            // component's: the component is whichever is among its own.
            let own = representations.get(&child);
            let is_own = |r: &u64| own.is_some_and(|own| own.contains(r));
            if !is_own(first) && is_own(second) {
                transform.inverted()
            } else {
                *transform
            }
        };

        struct Context<'s, 'a> {
            entities: &'s Entities<'a>,
            children: &'s BTreeMap<u64, Vec<(u64, u64)>>,
            bodies_of: &'s dyn Fn(u64) -> Vec<usize>,
            transform_for: &'s dyn Fn(u64, u64) -> Transform,
        }
        fn build(context: &Context, pd: u64, transform: Transform, path: &mut Vec<u64>) -> Product {
            path.push(pd);
            let mut children = Vec::new();
            for &(usage, child) in context.children.get(&pd).into_iter().flatten() {
                // A product that contains itself is a broken file, not an
                // infinitely deep tree.
                if path.contains(&child) || path.len() >= 64 {
                    continue;
                }
                children.push(build(context, child, (context.transform_for)(usage, child), path));
            }
            path.pop();
            Product {
                name: context.entities.definition_name(pd).unwrap_or_else(|| "Part".into()),
                transform,
                bodies: (context.bodies_of)(pd),
                children,
            }
        }
        let context = Context { entities: self, children: &children, bodies_of: &bodies_of, transform_for: &transform_for };
        let roots: Vec<u64> = shapes.keys().chain(children.keys()).copied().filter(|pd| !used.contains(pd)).collect();
        let mut roots: Vec<u64> = roots.into_iter().collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        roots.retain(|pd| self.is(*pd, "PRODUCT_DEFINITION"));
        let mut products: Vec<Product> =
            roots.into_iter().map(|pd| build(&context, pd, Transform::IDENTITY, &mut Vec::new())).collect();

        let mut claimed = std::collections::HashSet::new();
        fn claim(product: &Product, claimed: &mut std::collections::HashSet<usize>) {
            claimed.extend(product.bodies.iter().copied());
            product.children.iter().for_each(|c| claim(c, claimed));
        }
        products.iter().for_each(|p| claim(p, &mut claimed));
        let loose: Vec<usize> = (0..bodies.len()).filter(|b| !claimed.contains(b)).collect();
        if !loose.is_empty() {
            match products.first_mut() {
                Some(first) => first.bodies.extend(loose),
                None => products.push(Product {
                    name: fallback.to_string(),
                    transform: Transform::IDENTITY,
                    bodies: loose,
                    children: Vec::new(),
                }),
            }
        }
        products
    }

    /// What a `PRODUCT_DEFINITION_SHAPE` is the shape of: a product
    /// definition, or an assembly usage.
    fn shape_definition(&self, shape: u64) -> Option<u64> {
        arguments(self.record(shape, "PRODUCT_DEFINITION_SHAPE")?).get(2).and_then(reference)
    }

    /// A product definition's name: its product's.
    fn definition_name(&self, pd: u64) -> Option<String> {
        let formation = arguments(self.record(pd, "PRODUCT_DEFINITION")?).get(2).and_then(reference)?;
        let formation = self
            .record(formation, "PRODUCT_DEFINITION_FORMATION")
            .or_else(|| self.record(formation, "PRODUCT_DEFINITION_FORMATION_WITH_SPECIFIED_SOURCE"))?;
        let product = arguments(formation).get(2).and_then(reference)?;
        let args = arguments(self.record(product, "PRODUCT")?);
        [1, 0].into_iter().filter_map(|i| args.get(i).and_then(string)).find(|s| !s.trim().is_empty())
    }

    /// A representation relationship with a transformation: its two
    /// representations, and the transform taking the first's frame into the
    /// second's.
    fn placement_of(&self, relation: u64) -> Option<(u64, u64, Transform)> {
        let args = arguments(self.record(relation, "REPRESENTATION_RELATIONSHIP")?);
        let first = args.get(2).and_then(reference)?;
        let second = args.get(3).and_then(reference)?;
        let with = self.record(relation, "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION")?;
        let transformation = arguments(with).first().and_then(reference)?;
        let items = arguments(self.record(transformation, "ITEM_DEFINED_TRANSFORMATION")?);
        let from = self.axis_placement(items.get(2).and_then(reference)?)?;
        let to = self.axis_placement(items.get(3).and_then(reference)?)?;
        Some((first, second, to * from.inverted()))
    }

    /// An `AXIS2_PLACEMENT_3D` as the transform from its frame to its
    /// representation's.
    fn axis_placement(&self, id: u64) -> Option<Transform> {
        let args = arguments(self.record(id, "AXIS2_PLACEMENT_3D")?);
        let location = self.coordinates(args.get(1).and_then(reference)?, "CARTESIAN_POINT")?;
        let direction = |i: usize| {
            args.get(i)
                .and_then(reference)
                .and_then(|d| self.coordinates(d, "DIRECTION"))
                .and_then(|v| normalized(v))
        };
        let z = direction(2).unwrap_or([0.0, 0.0, 1.0]);
        // ISO 10303-42's first_proj_axis, as the vendored truck-stepio fix.
        let reference = direction(3).unwrap_or(if z[1].abs() < 1e-9 && z[2].abs() < 1e-9 {
            [0.0, 1.0, 0.0]
        } else {
            [1.0, 0.0, 0.0]
        });
        let along = dot(reference, z);
        let x = normalized([reference[0] - along * z[0], reference[1] - along * z[1], reference[2] - along * z[2]])?;
        let y = [z[1] * x[2] - z[2] * x[1], z[2] * x[0] - z[0] * x[2], z[0] * x[1] - z[1] * x[0]];
        let mut m = Mat4::IDENTITY;
        for row in 0..3 {
            m.m[row][0] = x[row] as f32;
            m.m[row][1] = y[row] as f32;
            m.m[row][2] = z[row] as f32;
            m.m[row][3] = location[row] as f32;
        }
        Transform::from_matrix(m)
    }

    /// The three numbers of a `CARTESIAN_POINT` or `DIRECTION`.
    fn coordinates(&self, id: u64, kind: &str) -> Option<[f64; 3]> {
        let Parameter::List(values) = arguments(self.record(id, kind)?).get(1)? else { return None };
        let mut out = [0.0; 3];
        for (slot, value) in out.iter_mut().zip(values) {
            *slot = real(value)?;
        }
        Some(out)
    }

    /// An `ORIENTED_SHELL` as the shell it wraps and whether it is
    /// reversed; `None` for anything else.
    fn oriented_shell(&self, id: u64) -> Option<(u64, bool)> {
        let record = self.record(id, "ORIENTED_SHELL")?;
        let args = arguments(record);
        let shell = args.get(2).and_then(reference)?;
        let reversed = matches!(args.get(3), Some(Parameter::Enumeration(e)) if e == "F");
        Some((shell, reversed))
    }

    /// The colour each styled item is drawn in, by the item's id.
    fn colours(&self) -> HashMap<u64, Colour> {
        let mut colours = HashMap::new();
        for kind in ["STYLED_ITEM", "OVER_RIDING_STYLED_ITEM"] {
            for (_, styled) in self.named(kind) {
                let args = arguments(styled);
                let (Some(styles), Some(item)) = (args.get(1), args.get(2).and_then(reference)) else { continue };
                if let Some(colour) = references(styles).into_iter().find_map(|s| self.surface_colour(s, 0)) {
                    // An overriding style is listed after what it overrides
                    // in every file seen; it wins either way.
                    if kind == "OVER_RIDING_STYLED_ITEM" || !colours.contains_key(&item) {
                        colours.insert(item, colour);
                    }
                }
            }
        }
        colours
    }

    /// Follow a presentation style down to the colour its surfaces fill
    /// with. Only the surface chain: a curve style's colour is the edges'.
    fn surface_colour(&self, id: u64, depth: usize) -> Option<Colour> {
        const CHAIN: [&str; 7] = [
            "PRESENTATION_STYLE_ASSIGNMENT",
            "PRESENTATION_STYLE_BY_CONTEXT",
            "SURFACE_STYLE_USAGE",
            "SURFACE_SIDE_STYLE",
            "SURFACE_STYLE_FILL_AREA",
            "FILL_AREA_STYLE",
            "FILL_AREA_STYLE_COLOUR",
        ];
        if depth > CHAIN.len() + 1 {
            return None;
        }
        if let Some(rgb) = self.record(id, "COLOUR_RGB") {
            let args = arguments(rgb);
            let channel = |i: usize| args.get(i).and_then(real).map(|v| v as f32);
            return Some(Colour::Rgb([channel(1)?, channel(2)?, channel(3)?]));
        }
        if let Some(named) = self.record(id, "DRAUGHTING_PRE_DEFINED_COLOUR") {
            return arguments(named).first().and_then(string).and_then(|n| predefined(&n));
        }
        let records = self.by_id.get(&id)?;
        let record = records.iter().find(|r| CHAIN.contains(&r.name.as_str()))?;
        arguments(record)
            .iter()
            .flat_map(references)
            .find_map(|next| self.surface_colour(next, depth + 1))
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalized(v: [f64; 3]) -> Option<[f64; 3]> {
    let length = dot(v, v).sqrt();
    (length > 1e-12 && length.is_finite()).then(|| [v[0] / length, v[1] / length, v[2] / length])
}

/// The eight colours `DRAUGHTING_PRE_DEFINED_COLOUR` may name.
fn predefined(name: &str) -> Option<Colour> {
    let (name, rgb): (&'static str, [f32; 3]) = match name.trim().to_ascii_lowercase().as_str() {
        "black" => ("black", [0.0, 0.0, 0.0]),
        "red" => ("red", [1.0, 0.0, 0.0]),
        "green" => ("green", [0.0, 1.0, 0.0]),
        "blue" => ("blue", [0.0, 0.0, 1.0]),
        "yellow" => ("yellow", [1.0, 1.0, 0.0]),
        "magenta" => ("magenta", [1.0, 0.0, 1.0]),
        "cyan" => ("cyan", [0.0, 1.0, 1.0]),
        "white" => ("white", [1.0, 1.0, 1.0]),
        _ => return None,
    };
    Some(Colour::Named(name, rgb))
}

/// The predefined colours a STEP file names. OpenCASCADE hands back every
/// colour as RGB, so these are what let its pure blue be called "blue" where
/// the file said `DRAUGHTING_PRE_DEFINED_COLOUR('blue')`, as truck's reading
/// calls it, while a `COLOUR_RGB` red stays "Colour #FF0000".
fn predefined_in(bytes: &[u8]) -> Vec<Colour> {
    const KEY: &[u8] = b"DRAUGHTING_PRE_DEFINED_COLOUR";
    let mut found: Vec<Colour> = Vec::new();
    let mut rest = bytes;
    while let Some(at) = rest.windows(KEY.len()).position(|w| w == KEY) {
        rest = &rest[at + KEY.len()..];
        let open = rest.iter().position(|&b| b == b'\'').filter(|&i| i < 8);
        let named = open.and_then(|i| {
            let tail = &rest[i + 1..];
            let close = tail.iter().take(16).position(|&b| b == b'\'')?;
            predefined(std::str::from_utf8(&tail[..close]).ok()?)
        });
        if let Some(colour) = named.filter(|c| !found.iter().any(|f| f.name() == c.name())) {
            found.push(colour);
        }
    }
    found
}

/// A record's arguments. The parser keeps a simple record's parameter list
/// as one `List`.
pub(crate) fn arguments(record: &Record) -> &[Parameter] {
    match &record.parameter {
        Parameter::List(list) => list,
        other => std::slice::from_ref(other),
    }
}

pub(crate) fn reference(parameter: &Parameter) -> Option<u64> {
    match parameter {
        Parameter::Ref(Name::Entity(id)) => Some(*id),
        Parameter::Typed { parameter, .. } => reference(parameter),
        _ => None,
    }
}

/// Every entity a parameter refers to, directly or in a list.
pub(crate) fn references(parameter: &Parameter) -> Vec<u64> {
    match parameter {
        Parameter::List(list) => list.iter().filter_map(reference).collect(),
        other => reference(other).into_iter().collect(),
    }
}

pub(crate) fn real(parameter: &Parameter) -> Option<f64> {
    match parameter {
        Parameter::Real(v) => Some(*v),
        Parameter::Integer(v) => Some(*v as f64),
        Parameter::Typed { parameter, .. } => real(parameter),
        _ => None,
    }
}

/// A string argument, with STEP's escapes for characters outside ASCII
/// decoded: `\X2\00E9\X0\` is é, the form CAD packages write names in.
pub(crate) fn string(parameter: &Parameter) -> Option<String> {
    let Parameter::String(raw) = parameter else { return None };
    Some(decode(raw))
}

fn decode(raw: &str) -> String {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find('\\') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        if let Some(body) = rest.strip_prefix("\\X2\\").or_else(|| rest.strip_prefix("\\X4\\")) {
            let width = if rest.starts_with("\\X2\\") { 4 } else { 8 };
            let end = body.find("\\X0\\").unwrap_or(body.len());
            let hex = &body[..end];
            if width == 4 {
                let units: Vec<u16> = (0..hex.len() / 4).filter_map(|i| u16::from_str_radix(&hex[i * 4..i * 4 + 4], 16).ok()).collect();
                out.extend(char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')));
            } else {
                out.extend((0..hex.len() / 8).filter_map(|i| u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16).ok()).map(|c| char::from_u32(c).unwrap_or('\u{FFFD}')));
            }
            rest = body.get(end + 4..).unwrap_or("");
        } else if let Some(body) = rest.strip_prefix("\\X\\") {
            // One ISO 8859-1 byte, as two hex digits.
            match body.get(..2).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(byte) => {
                    out.push(byte as char);
                    rest = &body[2..];
                }
                None => {
                    out.push('\\');
                    rest = &rest[1..];
                }
            }
        } else if let Some(body) = rest.strip_prefix("\\\\") {
            out.push('\\');
            rest = body;
        } else {
            out.push('\\');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod face_alone_tests {
    use super::*;

    /// A face divided alone carries only its own edges and their end
    /// points, whatever the shell around it, and divides into exactly the
    /// triangles it did with the whole shell's copied in: the retry costs a
    /// face's size, not the shell's times the face count.
    #[test]
    fn a_face_alone_is_only_its_own_edges_and_divides_the_same() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/two_bodies.step");
        let text = std::fs::read_to_string(path).unwrap();
        let data = crate::part21::data_section(&text).unwrap();
        let entities = Entities::new(&data.entities);
        let table = Table::from_data_section(&data);
        let colours = entities.colours();
        let mut checked = 0;
        for body in entities.bodies(&table) {
            for (_, faces, _) in body.groups(&table, &colours) {
                let compressed = table.to_compressed_shell(&ShellHolder { label: String::new(), cfs_faces: faces }).unwrap();
                for i in 0..compressed.faces.len() {
                    let alone = face_alone(&compressed, i).expect("every edge of the fixture is read");
                    let used: std::collections::HashSet<usize> =
                        compressed.faces[i].boundaries.iter().flatten().map(|e| e.index).collect();
                    assert_eq!(alone.edges.len(), used.len(), "face {i} took edges it does not use");
                    assert!(alone.vertices.len() <= 2 * used.len());
                    let whole = CompressedShell {
                        vertices: compressed.vertices.clone(),
                        edges: compressed.edges.clone(),
                        faces: vec![compressed.faces[i].clone()],
                    };
                    let (a, b) = (alone.robust_triangulation(0.01), whole.robust_triangulation(0.01));
                    let triangles = |shell: &truck_topology::compress::CompressedShell<_, _, Option<PolygonMesh>>| {
                        shell.faces.iter().filter_map(|f| f.surface.as_ref()).map(|m| m.faces().len()).sum::<usize>()
                    };
                    assert_eq!(triangles(&a), triangles(&b), "face {i} divided differently alone");
                    checked += 1;
                }
            }
        }
        assert!(checked > 4, "the fixture had only {checked} faces");
    }

    /// The fixture's first body as truck reads it, and the tolerance it is
    /// divided at.
    fn fixture_shell() -> StepShell {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/two_bodies.step");
        let text = std::fs::read_to_string(path).unwrap();
        let data = crate::part21::data_section(&text).unwrap();
        let entities = Entities::new(&data.entities);
        let table = Table::from_data_section(&data);
        let body = entities.bodies(&table).into_iter().next().unwrap();
        let holder = table.shell.get(&body.shells[0].0).unwrap();
        table.to_compressed_shell(holder).unwrap()
    }

    /// A face naming an edge truck left out is lost alone - `None` - rather
    /// than panicking outside every guard and stopping the whole import.
    #[test]
    fn a_face_with_a_missing_edge_is_none_not_a_panic() {
        let mut shell = fixture_shell();
        let missing = shell.edges.len() + 5;
        shell.faces[0].boundaries[0][0].index = missing;
        assert!(face_alone(&shell, 0).is_none());
        assert!(face_alone(&shell, 1).is_some(), "a whole face was refused too");
    }

    /// A B-spline surface's knots rescaled to 0..1 describe the same
    /// surface: the same points at the matching parameters.
    #[test]
    fn rescaled_knots_leave_the_surface_where_it_was() {
        use truck_stepio::r#in::alias::Surface;
        let (u0, u1) = (0.0055, 0.0064);
        use truck_geometry::prelude::{BSplineSurface, KnotVec};
        let knots = (KnotVec::bezier_knot(1), KnotVec::bezier_knot(1));
        let mut tiny = knots.clone();
        tiny.0.transform(u1 - u0, u0);
        let points = vec![vec![Point3::new(0.0, 0.0, 0.0), Point3::new(0.0, 0.02, 0.0)], vec![Point3::new(0.03, 0.0, 0.01), Point3::new(0.03, 0.02, 0.0)]];
        let surface = BSplineSurface::new(tiny, points);
        let original = surface.clone();
        let mut shell = fixture_shell();
        shell.faces.truncate(1);
        shell.faces[0].surface = Surface::BSplineSurface(Box::new(surface));
        normalize_knots(&mut shell);
        let Surface::BSplineSurface(rescaled) = &shell.faces[0].surface else { panic!("not a B-spline") };
        assert_eq!(rescaled.uknot_vec().range_length(), 1.0);
        for (s, t) in [(0.0, 0.0), (0.5, 0.25), (1.0, 1.0)] {
            let a = rescaled.subs(s, t);
            let b = original.subs(u0 + s * (u1 - u0), t);
            assert!(a.distance(b) < 1e-12, "moved at ({s}, {t}): {a:?} against {b:?}");
        }
    }

    /// A division far bigger than its face's boundary - what truck made of
    /// a cylinder or a plane in a file in metres - is not kept: a plane is
    /// triangulated from its outline, filling no more than it, and any other
    /// surface's division is cut back to its boundary's box.
    #[test]
    fn a_division_bigger_than_its_boundary_is_settled_back_inside_it() {
        let shell = fixture_shell();
        let tolerance = 0.01;
        let mut planes = 0;
        for index in 0..shell.faces.len() {
            let Some(loops) = outline(&shell, index, tolerance) else { continue };
            let width = extent(span(loops.iter().flatten()).unwrap());
            let alone = face_alone(&shell, index).unwrap();
            let good = alone.robust_triangulation(tolerance).faces.into_iter().next().unwrap().surface.unwrap();
            // The same division with a sheet a hundred widths across added.
            let mut bad = good.clone();
            let far = Point3::new(100.0 * width, 0.0, 0.0);
            let sheet = PolygonMesh::new(
                StandardAttributes { positions: vec![good.positions()[0], far, Point3::new(0.0, 100.0 * width, 0.0)], uv_coords: vec![], normals: vec![] },
                Faces::from_iter([[0usize, 1, 2]]),
            );
            bad.merge(sheet);
            assert!(extent(span(bad.positions().iter()).unwrap()) > 50.0 * width);
            let kept = settle(&shell, index, Some(good.clone()), tolerance).unwrap();
            assert_eq!(kept.faces().len(), good.faces().len(), "face {index}: a division that fits was changed");
            let settled = settle(&shell, index, Some(bad), tolerance).expect("the face was lost");
            let used: Vec<Point3> = settled.faces().triangle_iter().flatten().map(|v| settled.positions()[v.pos]).collect();
            assert!(!used.is_empty());
            assert!(extent(span(used.iter()).unwrap()) <= 1.5 * width + 10.0 * tolerance, "face {index} still reaches past its boundary");
            planes += usize::from(flat_face(&shell, index, &loops).is_some());
        }
        assert!(planes > 0, "the fixture has no plane to triangulate flat");
    }
}
