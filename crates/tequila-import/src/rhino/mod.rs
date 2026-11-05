//! Rhino's `.3dm` (Rhino 5 to 8), read in pure Rust.
//!
//! The format is McNeel's openNURBS archive, whose reader is open source
//! (github.com/mcneel/opennurbs, MIT-style licence); this follows its field
//! order and version gates, read from that source, and links nothing of it.
//! No crate on crates.io was worth taking instead: the one 3dm reader there
//! that is pure Rust (`cadmpeg-codec-rhino`, Apache-2.0) decodes into a
//! neutral CAD model of its own through a hundred thousand lines and a
//! dozen dependencies, where Tequila needs triangles, names and materials.
//!
//! What comes in:
//!
//! * the start and version (Rhino 5's 32-bit and 64-bit archives, 6, 7 and
//!   8; Rhino 2 to 4 where their objects are the same), and the chunks;
//! * the settings' model unit and absolute tolerance;
//! * layers: names (as their full path, `Parent::Child`), colours, whether
//!   they are on (a sublayer of a layer that is off is off), their render
//!   material; each object is grouped under its layer and keeps the layer's
//!   name, so the Scene tab's Layers list shows and hides it;
//! * materials: Rhino's basic material and its PBR settings, as OpenPBR
//!   (`material.rs` says how), with their colour, bump, transparency and
//!   PBR maps;
//! * objects: meshes (plain and compressed arrays, normals, texture
//!   coordinates, vertex colours, double precision vertices); B-reps,
//!   from the render meshes Rhino saved with them where it did, else each
//!   face divided from its NURBS, plane, revolved or sum surface and its
//!   trims by the IGES divider (`crate::iges::divide_parametric`) at the
//!   import's quality; extrusions, from their saved meshes or divided the
//!   same way (sides and caps); SubD, from its control cage
//!   (`subd.rs`); lone surfaces; block definitions and the blocks that
//!   place them, each definition meshed once and placed as often as it
//!   is used, nested blocks too;
//! * each object's name, its render material or display colour (by layer,
//!   by object, by material or by parent, as Rhino resolves them), and
//!   whether it is hidden;
//! * B-rep edges, seams left out, for View > Model edges.
//!
//! Not read, and said so in the import's warnings: curves, points, point
//! clouds, annotation, hatches, lights, linked blocks (their file is not
//! opened), embedded bitmaps (a material's map is looked for where its
//! path says, then beside the model), mitered extrusion ends (drawn
//! square), RDK render content beyond what its simulated material says,
//! per-face colours and materials, texture mapping other than the mesh's
//! own coordinates, and offset surfaces and curve proxies in a B-rep.
//!
//! Rhino draws Z up; the model is turned onto Tequila's Y up on its top
//! node, so Rhino's Front view is Tequila's front.

mod archive;
mod geometry;
mod material;
mod subd;

use crate::raw::{RawMesh, RawNode, RawScene};
use crate::step::Tessellation;
use crate::ImportError;
use archive::{uuid, Reader, Uuid, NIL, R};
use geometry::{Curve, MeshData, Surface, Trim};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use tequila_core::math::{vec2, vec3, vec4, Mat4, Transform, Vec3};
use tequila_core::units::LengthUnit;

const ON_LAYER: Uuid = uuid("95809813-E985-11d3-BFE5-0010830122F0");
const ON_INSTANCE_DEFINITION: Uuid = uuid("26F8BFF6-2618-417f-A158-153D64A94989");
const ON_INSTANCE_REF: Uuid = uuid("F9CFB638-B9D4-4340-87E3-C56E7865D96A");

/// How deep blocks may nest in blocks.
const DEEPEST: usize = 16;

/// Whether the bytes are a 3dm file: every one starts by saying so.
pub fn looks_like(bytes: &[u8]) -> bool {
    bytes.starts_with(b"3D Geometry File Format ")
}

/// Read a 3dm file at the default quality.
pub fn read(bytes: &[u8], name: &str) -> Result<RawScene, ImportError> {
    read_with(bytes, name, Tessellation::default())
}

/// Read a 3dm file, dividing whatever has no saved mesh at `quality`.
pub fn read_with(bytes: &[u8], name: &str, quality: Tessellation) -> Result<RawScene, ImportError> {
    let file = parse(bytes)?;
    Ok(build(file, name, quality))
}

// The file.

#[derive(Clone, Debug, Default)]
struct Layer {
    name: String,
    colour: [u8; 4],
    visible: bool,
    material: i32,
    id: Uuid,
    parent: Uuid,
}

/// The parts of `ON_3dmObjectAttributes` that decide how an object looks.
#[derive(Clone, Debug)]
struct Attributes {
    id: Uuid,
    layer: i32,
    name: String,
    material: i32,
    colour: [u8; 4],
    visible: bool,
    /// `ON::object_mode`: 0 normal, 1 hidden, 2 locked, 3 in a block.
    mode: u8,
    /// 0 by layer, 1 by object, 2 by material, 3 by parent.
    colour_source: u8,
    /// 0 by layer, 1 by object, 3 by parent.
    material_source: u8,
}

impl Default for Attributes {
    fn default() -> Self {
        Attributes { id: NIL, layer: 0, name: String::new(), material: -1, colour: [0; 4], visible: true, mode: 0, colour_source: 0, material_source: 0 }
    }
}

enum Geometry {
    Mesh(MeshData),
    Brep(geometry::Brep),
    Extrusion(geometry::Extrusion),
    SubD(subd::Cage),
    Surface(Surface),
    Block { definition: Uuid, xform: [[f64; 4]; 4] },
}

struct Record {
    attributes: Attributes,
    geometry: Geometry,
}

struct Definition {
    name: String,
    members: Vec<Uuid>,
    linked: bool,
}

#[derive(Default)]
struct File {
    /// Metres per unit, and the unit Tequila has a name for, when the file
    /// says one.
    unit: Option<(f64, LengthUnit)>,
    unit_name: String,
    tolerance: f64,
    layers: Vec<Layer>,
    materials: Vec<Option<material::RhinoMaterial>>,
    definitions: HashMap<Uuid, Definition>,
    records: Vec<Record>,
    /// Kinds of object met and not read, with how many.
    skipped: BTreeMap<String, usize>,
    warnings: Vec<String>,
}

fn malformed(e: String) -> ImportError {
    ImportError::Malformed(format!("this is not a 3dm file Tequila can read: {e}"))
}

fn parse(bytes: &[u8]) -> Result<File, ImportError> {
    if !looks_like(bytes) || bytes.len() < 32 {
        return Err(malformed("it does not start as a 3dm file does".into()));
    }
    let version: u32 = std::str::from_utf8(&bytes[24..32])
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .ok_or_else(|| malformed("its version is not a number".into()))?;
    if version < 2 {
        return Err(ImportError::Unsupported("this 3dm file is from Rhino 1, whose format Tequila does not read; save it again from a later Rhino".into()));
    }
    if !(version <= 5 || version % 10 == 0) {
        return Err(malformed(format!("its version, {version}, is not one Rhino writes")));
    }
    let mut file = File::default();
    let mut top = Reader::new(bytes, 32, version);
    let mut objects_seen = false;
    while top.remaining() > 0 {
        let chunk = match top.chunk() {
            Ok(c) => c,
            // A file cut short after its objects is still its objects.
            Err(e) if objects_seen => {
                file.warnings.push(format!("the file ends early ({e}); everything before that is read"));
                break;
            }
            Err(e) => return Err(malformed(e)),
        };
        match chunk.tcode {
            archive::TCODE_PROPERTIES_TABLE => {
                let mut t = chunk.body;
                while t.remaining() > 0 {
                    let Ok(c) = t.chunk() else { break };
                    if c.tcode == archive::TCODE_PROPERTIES_OPENNURBS_VERSION {
                        top.opennurbs = c.value.max(0) as u64;
                    }
                }
            }
            archive::TCODE_SETTINGS_TABLE => {
                let mut t = chunk.body;
                while t.remaining() > 0 {
                    let Ok(c) = t.chunk() else { break };
                    if c.tcode == archive::TCODE_SETTINGS_UNITSANDTOLS {
                        if let Err(e) = units(c.body, &mut file) {
                            file.warnings.push(format!("the model's units could not be read ({e})"));
                        }
                    }
                }
            }
            archive::TCODE_LAYER_TABLE => table(chunk.body, archive::TCODE_LAYER_RECORD, &mut file, |r, file| {
                let object = r.object()?;
                if object.class != ON_LAYER {
                    return Err("a layer record holds something else".into());
                }
                file.layers.push(layer(object.data)?);
                Ok(())
            }),
            archive::TCODE_MATERIAL_TABLE => table(chunk.body, archive::TCODE_MATERIAL_RECORD, &mut file, |r, file| {
                let object = r.object()?;
                // A material that does not read keeps its place: objects
                // name materials by where they are in the table.
                let read = (object.class == material::ON_MATERIAL).then(|| material::read(&object));
                match read {
                    Some(Ok(m)) => file.materials.push(Some(m)),
                    Some(Err(e)) => {
                        file.materials.push(None);
                        return Err(e);
                    }
                    None => file.materials.push(None),
                }
                Ok(())
            }),
            archive::TCODE_INSTANCE_DEFINITION_TABLE => table(chunk.body, archive::TCODE_INSTANCE_DEFINITION_RECORD, &mut file, |r, file| {
                let object = r.object()?;
                if object.class != ON_INSTANCE_DEFINITION {
                    return Err("a block definition record holds something else".into());
                }
                let (id, d) = definition(object.data)?;
                file.definitions.insert(id, d);
                Ok(())
            }),
            archive::TCODE_OBJECT_TABLE => {
                objects_seen = true;
                table(chunk.body, archive::TCODE_OBJECT_RECORD, &mut file, object_record);
            }
            archive::TCODE_ENDOFFILE => break,
            _ => {}
        }
    }
    Ok(file)
}

/// The records of a table, each read by `read`; one that does not read is
/// a warning, and the rest still come in.
fn table(mut t: Reader<'_>, record: u32, file: &mut File, read: impl Fn(&mut Reader<'_>, &mut File) -> R<()>) {
    let mut failed: Vec<String> = Vec::new();
    while t.remaining() > 0 {
        let chunk = match t.chunk() {
            Ok(c) => c,
            Err(e) => {
                failed.push(e);
                break;
            }
        };
        if chunk.tcode == archive::TCODE_ENDOFTABLE {
            break;
        }
        if chunk.tcode == record {
            let mut body = chunk.body;
            if let Err(e) = read(&mut body, file) {
                failed.push(e);
            }
        }
    }
    if let Some(first) = failed.first() {
        file.warnings.push(format!("{} records could not be read and were left out (the first: {first})", failed.len()));
    }
}

/// `ON_3dmUnitsAndTolerances`: a version, the unit system, the absolute,
/// angle and relative tolerances, and (from version 102) the metres in a
/// custom unit and its name.
fn units(mut r: Reader<'_>, file: &mut File) -> R<()> {
    let version = r.i32()?;
    let system = r.i32()?;
    file.tolerance = r.f64()?;
    r.skip(16)?;
    if version >= 101 {
        r.skip(8)?;
    }
    let custom = if version >= 102 { Some((r.f64()?, r.string()?)) } else { None };
    let (metres, name): (f64, &str) = match system {
        1 => (1e-6, "microns"),
        2 => (1e-3, "millimetres"),
        3 => (1e-2, "centimetres"),
        4 => (1.0, "metres"),
        5 => (1e3, "kilometres"),
        6 => (2.54e-8, "microinches"),
        7 => (2.54e-5, "mils"),
        8 => (0.0254, "inches"),
        9 => (0.3048, "feet"),
        10 => (1609.344, "miles"),
        11 => match &custom {
            Some((m, _)) if *m > 0.0 && m.is_finite() => (*m, "custom units"),
            _ => return Ok(()),
        },
        12 => (1e-10, "ångströms"),
        13 => (1e-9, "nanometres"),
        14 => (0.1, "decimetres"),
        15 => (10.0, "decametres"),
        16 => (100.0, "hectometres"),
        17 => (1e6, "megametres"),
        18 => (1e9, "gigametres"),
        19 => (0.9144, "yards"),
        20 => (0.0254 / 72.0, "points"),
        21 => (0.0254 / 6.0, "picas"),
        22 => (1852.0, "nautical miles"),
        23 => (1.495_978_707e11, "astronomical units"),
        24 => (9.460_730_472_580_8e15, "light years"),
        25 => (3.085_677_581_49e16, "parsecs"),
        // None, or unset: the bounding box is guessed from.
        _ => return Ok(()),
    };
    let known = match system {
        2 => LengthUnit::Millimetre,
        3 => LengthUnit::Centimetre,
        8 => LengthUnit::Inch,
        9 => LengthUnit::Foot,
        _ => LengthUnit::Metre,
    };
    file.unit = Some((metres, known));
    file.unit_name = match (system, &custom) {
        (11, Some((_, n))) if !n.is_empty() => n.clone(),
        _ => name.to_string(),
    };
    Ok(())
}

/// `ON_Layer::Read`, as far as its parent: a version byte, an obsolete
/// mode, its index, IGES level, render material, an obsolete number, its
/// colour, obsolete line style and widths, its name, whether it is on
/// (version 1.1), its line type, plot colour and weight, whether it is
/// locked, its id (1.5) and its parent's (1.6).
fn layer(mut r: Reader<'_>) -> R<Layer> {
    let (major, minor) = r.version()?;
    if major != 1 {
        return Err(format!("a layer of version {major}"));
    }
    let mode = r.i32()?;
    r.i32()?;
    r.i32()?;
    let material = r.i32()?;
    r.i32()?;
    let colour = [r.u8()?, r.u8()?, r.u8()?, r.u8()?];
    r.skip(4 + 16)?;
    let name = r.string()?;
    let visible = if minor >= 1 { r.bool()? } else { mode != 1 };
    let mut l = Layer { name, colour, visible, material, id: NIL, parent: NIL };
    if minor >= 2 {
        r.i32()?;
    }
    if minor >= 3 {
        r.skip(4 + 8)?;
    }
    if minor >= 4 {
        r.bool()?;
    }
    if minor >= 5 {
        l.id = r.uuid()?;
    }
    if minor >= 6 && r.opennurbs > 200_505_110 {
        l.parent = r.uuid()?;
    }
    Ok(l)
}

/// `ON_InstanceDefinition::Read`: from Rhino 6 an anonymous chunk with the
/// component's attributes, its kind, units, description, URL and box, then
/// its members' ids behind a flag; before, a version byte, its id, its
/// members, its name and the rest.
fn definition(mut r: Reader<'_>) -> R<(Uuid, Definition)> {
    let modern = r.archive >= 60 && r.remaining() > 0 && { r }.u8()? == 0;
    if modern {
        let (mut b, major, _) = r.anonymous("a block definition")?;
        if major != 1 {
            return Err(format!("a block definition of version {major}"));
        }
        let (_, id, name) = material::component(&mut b)?;
        let kind = b.u32()?;
        b.chunk()?;
        for _ in 0..3 {
            b.string()?;
        }
        b.bbox()?;
        let members = if b.bool()? { uuids(&mut b)? } else { Vec::new() };
        return Ok((id, Definition { name, members, linked: kind == 3 }));
    }
    let (major, _) = r.version()?;
    if major != 1 {
        return Err(format!("a block definition of version {major}"));
    }
    let id = r.uuid()?;
    let members = uuids(&mut r)?;
    let name = r.string()?;
    for _ in 0..3 {
        r.string()?;
    }
    r.bbox()?;
    let kind = r.u32().unwrap_or(0);
    Ok((id, Definition { name, members, linked: kind == 3 }))
}

fn uuids(r: &mut Reader<'_>) -> R<Vec<Uuid>> {
    let n = r.count(16)?;
    (0..n).map(|_| r.uuid()).collect()
}

/// `ON_3dmObjectAttributes::Read`. Version 1 is a fixed list; version 2
/// (Rhino 5 on) its id and layer, then items each led by a number, in
/// rising order, ended by a zero. Items past the ones read here end the
/// reading: they have no length to skip by, and nothing after them
/// matters to a render.
fn attributes(mut r: Reader<'_>) -> R<Attributes> {
    let (major, minor) = r.version()?;
    let mut a = Attributes::default();
    let colour = |r: &mut Reader<'_>| -> R<[u8; 4]> { Ok([r.u8()?, r.u8()?, r.u8()?, r.u8()?]) };
    match major {
        1 => {
            a.id = r.uuid()?;
            a.layer = r.i32()?;
            a.material = r.i32()?;
            a.colour = colour(&mut r)?;
            r.skip(4 + 16 + 4)?;
            a.mode = r.u8()? & 0x0F;
            a.colour_source = r.u8()?;
            r.u8()?;
            a.material_source = r.u8()?;
            a.name = r.string()?;
            r.string()?;
            if minor >= 1 {
                r.int_array()?;
            }
            a.visible = if minor >= 2 { r.bool()? } else { a.mode != 1 };
        }
        2 => {
            a.id = r.uuid()?;
            a.layer = r.i32()?;
            let mut last = 0u8;
            while r.remaining() > 0 {
                let item = r.u8()?;
                if item == 0 || item <= last {
                    break;
                }
                last = item;
                match item {
                    1 => a.name = r.string()?,
                    2 => {
                        r.string()?;
                    }
                    3 => {
                        r.i32()?;
                    }
                    4 => a.material = r.i32()?,
                    5 => {
                        r.chunk()?;
                    }
                    6 => a.colour = colour(&mut r)?,
                    7 => r.skip(4)?,
                    8 => r.skip(8)?,
                    9 => r.skip(1)?,
                    10 => r.skip(4)?,
                    11 => a.visible = r.bool()?,
                    12 => a.mode = r.u8()? & 0x0F,
                    13 => a.colour_source = r.u8()?,
                    14 | 15 => r.skip(1)?,
                    16 => a.material_source = r.u8()?,
                    _ => break,
                }
            }
        }
        v => return Err(format!("object attributes of version {v}")),
    }
    Ok(a)
}

/// One object record: its type, the object, its attributes.
fn object_record(r: &mut Reader<'_>, file: &mut File) -> R<()> {
    let mut object = None;
    let mut attrs = None;
    while r.remaining() > 0 {
        if r.peek_u32() == Some(archive::TCODE_OPENNURBS_CLASS) {
            object = Some(r.object()?);
            continue;
        }
        let chunk = r.chunk()?;
        match chunk.tcode {
            archive::TCODE_OBJECT_RECORD_ATTRIBUTES => attrs = Some(attributes(chunk.body)?),
            archive::TCODE_OBJECT_RECORD_END => break,
            _ => {}
        }
    }
    let object = object.ok_or("an object record with no object")?;
    let attributes = attrs.unwrap_or_default();
    let what = |e: String, kind: &str| format!("{kind} `{}`: {e}", if attributes.name.is_empty() { "unnamed" } else { &attributes.name });
    let geometry = match object.class {
        geometry::ON_MESH => Geometry::Mesh(geometry::mesh(&object).map_err(|e| what(e, "a mesh"))?),
        geometry::ON_BREP => Geometry::Brep(geometry::brep(&object).map_err(|e| what(e, "a polysurface"))?),
        geometry::ON_EXTRUSION => Geometry::Extrusion(geometry::extrusion(&object).map_err(|e| what(e, "an extrusion"))?),
        subd::ON_SUBD => match subd::subd(&object).map_err(|e| what(e, "a SubD"))? {
            Some(cage) => Geometry::SubD(cage),
            None => return Ok(()),
        },
        ON_INSTANCE_REF => {
            let mut d = object.data;
            let (major, _) = d.version()?;
            if major != 1 {
                return Err(format!("a block of version {major}"));
            }
            let definition = d.uuid()?;
            let xform = d.xform()?;
            Geometry::Block { definition, xform }
        }
        geometry::ON_NURBS_SURFACE | geometry::ON_PLANE_SURFACE | geometry::ON_REV_SURFACE | geometry::ON_SUM_SURFACE => {
            match geometry::surface(&object).map_err(|e| what(e, "a surface"))? {
                Some(s) => Geometry::Surface(s),
                None => return Ok(()),
            }
        }
        class => {
            *file.skipped.entry(kind_of(class)).or_default() += 1;
            return Ok(());
        }
    };
    file.records.push(Record { attributes, geometry });
    Ok(())
}

/// What an object not read is, for the warning that lists them.
fn kind_of(class: Uuid) -> String {
    let curves = [geometry::ON_NURBS_CURVE, geometry::ON_LINE_CURVE, geometry::ON_ARC_CURVE, geometry::ON_POLYLINE_CURVE, geometry::ON_POLY_CURVE];
    if curves.contains(&class) {
        return "curves".into();
    }
    // The classes of what a file draws and Tequila does not render, current
    // and retired, as openNURBS names them.
    const KINDS: [(Uuid, &str); 29] = [
        (uuid("C3101A1D-F157-11d3-BFE7-0010830122F0"), "points"),
        (uuid("2488F347-F8FA-11d3-BFEC-0010830122F0"), "point clouds"),
        (uuid("4ED7D4E5-E947-11d3-BFE5-0010830122F0"), "point grids"),
        (uuid("57376349-62A9-4A16-B411-A46BCD544790"), "text"),
        (uuid("46F75541-F46B-48BE-AA7E-B353BBE068A7"), "text"),
        (uuid("5DE6B210-486B-11d4-8014-0010830122F0"), "text"),
        (uuid("945BF594-6FF9-4F5C-BFC0-B3AF528F29D2"), "leaders"),
        (uuid("14922B7A-5B65-4F11-8345-D415A9637129"), "leaders"),
        (uuid("5DE6B211-486B-11d4-8014-0010830122F0"), "leaders"),
        (uuid("E550882B-F44D-4154-A1EF-6E50CBBBF543"), "dimensions"),
        (uuid("BD57F33B-A1B2-46E9-9C6E-AF09D30FFDDE"), "dimensions"),
        (uuid("5DE6B20D-486B-11d4-8014-0010830122F0"), "dimensions"),
        (uuid("D417786B-F6CD-4F12-9E1F-063F414DBEB6"), "dimensions"),
        (uuid("841BC40B-A971-4A8E-94E5-BBA26D67348E"), "dimensions"),
        (uuid("5DE6B20F-486B-11d4-8014-0010830122F0"), "dimensions"),
        (uuid("FC749C2F-4C00-41FD-9840-26D94F047AD3"), "dimensions"),
        (uuid("B2B683FC-7964-4E96-B1F9-9B356A76B08B"), "dimensions"),
        (uuid("5DE6B20E-486B-11d4-8014-0010830122F0"), "dimensions"),
        (uuid("03124828-4C9B-4D28-9A82-664DDDE7A14F"), "dimensions"),
        (uuid("C8288D69-5BD8-4F50-9BAF-525A0086B0C3"), "dimensions"),
        (uuid("D46767BA-7E8F-4D9D-9A92-66050219A5B9"), "centre marks"),
        (uuid("ABAF5873-4145-11d4-800F-0010830122F0"), "annotation"),
        (uuid("8BD94E1A-59E1-11d4-8018-0010830122F0"), "annotation"),
        (uuid("74198302-CDF4-4F95-9609-6D684F22AB37"), "text dots"),
        (uuid("8BD94E19-59E1-11d4-8018-0010830122F0"), "text dots"),
        (uuid("0559733B-5332-49D1-A936-0532AC76ADE5"), "hatches"),
        (uuid("C8C66EFA-B3CB-4E00-9440-2AD66203379E"), "details"),
        (uuid("06936AFB-3D3C-41AC-BF70-C9319FA480A1"), "cages"),
        (uuid("D379E6D8-7C31-4407-A913-E3B7040D034A"), "morph controls"),
    ];
    match KINDS.iter().find(|(c, _)| *c == class) {
        Some((_, n)) => n.to_string(),
        None => geometry::class_name(class),
    }
}

// The scene.

/// A record turned into triangles, once, in the file's coordinates.
#[derive(Default)]
struct Built {
    mesh: MeshData,
    edges: Vec<[f64; 3]>,
}

/// A face to divide: its surface and loops, and which object it is for.
struct Job {
    record: usize,
    surface: Arc<Surface>,
    loops: Vec<Vec<Trim>>,
    reversed: bool,
    tolerance: f64,
}

fn build(file: File, name: &str, quality: Tessellation) -> RawScene {
    let File { unit, unit_name, tolerance: model_tolerance, layers, materials, definitions, records, skipped, mut warnings } = file;
    let scale = unit.map_or(1.0, |(metres, known)| metres / known.to_metres());

    // Which records sit at the top: not a member of any block and not
    // marked as one.
    let members: HashSet<Uuid> = definitions.values().flat_map(|d| d.members.iter().copied()).collect();
    let by_id: HashMap<Uuid, usize> = records.iter().enumerate().filter(|(_, r)| r.attributes.id != NIL).map(|(i, r)| (r.attributes.id, i)).collect();

    // How big the model is, for the floor under each part's tolerance.
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    let mut grow = |p: [f64; 3]| {
        if p.iter().all(|c| c.is_finite()) {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    };
    let mut sizes = vec![0.0f64; records.len()];
    for (i, r) in records.iter().enumerate() {
        let points: Vec<[f64; 3]> = match &r.geometry {
            // Enough of a mesh's vertices to size it, not a copy of them all.
            Geometry::Mesh(m) => m.positions.iter().step_by(m.positions.len() / 4096 + 1).copied().collect(),
            Geometry::Brep(b) => {
                let mut p = b.vertices.clone();
                for f in &b.faces {
                    if let Some(m) = &f.mesh {
                        p.extend(m.positions.iter().step_by(16));
                    } else if let Some(s) = &f.surface {
                        let d = s.domain();
                        for (u, v) in [(d[0], d[2]), (d[1], d[2]), (d[1], d[3]), (d[0], d[3]), ((d[0] + d[1]) / 2.0, (d[2] + d[3]) / 2.0)] {
                            p.push(s.at(u, v));
                        }
                    }
                }
                p
            }
            Geometry::Extrusion(e) => {
                let mut p = vec![e.start, e.end];
                for c in &e.profiles {
                    let d = c.domain();
                    for k in 0..=8 {
                        let q = c.at(d[0] + (d[1] - d[0]) * k as f64 / 8.0);
                        p.push([e.start[0] + e.x[0] * q[0] + e.y[0] * q[1], e.start[1] + e.x[1] * q[0] + e.y[1] * q[1], e.start[2] + e.x[2] * q[0] + e.y[2] * q[1]]);
                    }
                }
                p
            }
            Geometry::SubD(c) => c.points.clone(),
            Geometry::Surface(s) => {
                let d = s.domain();
                (0..=4).flat_map(|i| (0..=4).map(move |j| (i, j))).map(|(i, j)| s.at(d[0] + (d[1] - d[0]) * i as f64 / 4.0, d[2] + (d[3] - d[2]) * j as f64 / 4.0)).collect()
            }
            Geometry::Block { .. } => Vec::new(),
        };
        let (mut a, mut b) = ([f64::MAX; 3], [f64::MIN; 3]);
        for p in &points {
            grow(*p);
            for k in 0..3 {
                a[k] = a[k].min(p[k]);
                b[k] = b[k].max(p[k]);
            }
        }
        if !points.is_empty() {
            sizes[i] = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
        }
    }
    let model_size = if lo[0] <= hi[0] { ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt().max(1e-9) } else { 1.0 };
    let tolerance_for = |i: usize| (sizes[i] * quality.chord()).max(model_size * quality.chord() * 0.1).max(model_tolerance.max(0.0));

    // Everything there is to divide, divided at once and in parallel.
    let mut jobs = Vec::new();
    let mut mitered = 0usize;
    for (i, r) in records.iter().enumerate() {
        match &r.geometry {
            Geometry::Brep(b) => {
                for f in &b.faces {
                    if f.mesh.is_some() {
                        continue;
                    }
                    if let Some(s) = &f.surface {
                        jobs.push(Job { record: i, surface: s.clone(), loops: f.loops.clone(), reversed: f.reversed, tolerance: tolerance_for(i) });
                    }
                }
            }
            Geometry::Extrusion(e) if e.meshes.is_empty() => {
                if e.mitered {
                    mitered += 1;
                }
                for (s, loops, reversed) in e.faces() {
                    jobs.push(Job { record: i, surface: Arc::new(s), loops, reversed, tolerance: tolerance_for(i) });
                }
            }
            Geometry::Surface(s) => jobs.push(Job { record: i, surface: Arc::new(s.clone()), loops: Vec::new(), reversed: false, tolerance: tolerance_for(i) }),
            _ => {}
        }
    }
    let divided: Vec<(usize, Option<crate::iges::Divided>, [f64; 4])> = {
        use rayon::prelude::*;
        jobs.par_iter().map(|job| (job.record, crate::iges::divide_parametric(parametric(job), job.tolerance), job.surface.domain())).collect()
    };
    let divided_any = !jobs.is_empty();
    let mut failed = 0usize;
    let mut built: Vec<Built> = records.iter().map(|_| Built::default()).collect();
    for (record, piece, domain) in divided {
        let Some(piece) = piece else {
            failed += 1;
            continue;
        };
        let b = &mut built[record];
        let base = b.mesh.positions.len() as u32;
        b.mesh.positions.extend(piece.positions.iter().copied());
        b.mesh.normals.extend(piece.normals.iter().map(|n| [n[0] as f32, n[1] as f32, n[2] as f32]));
        // Each face's own parameters, made 0 to 1 over its surface, as
        // Rhino's surface parameter mapping has them.
        b.mesh.uvs.extend(piece.params.iter().map(|q| [((q[0] - domain[0]) / (domain[1] - domain[0])) as f32, ((q[1] - domain[2]) / (domain[3] - domain[2])) as f32]));
        b.mesh.triangles.extend(piece.triangles.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
        // Each face's boundary, which a B-rep with edge curves of its own
        // draws from those instead (below), once an edge rather than once
        // a face.
        b.edges.extend(piece.edges.iter().copied());
    }
    // The rest: saved meshes, SubD surfaces, B-rep edges.
    let levels = match quality {
        Tessellation::Draft => 2,
        Tessellation::Medium => 3,
        Tessellation::High | Tessellation::Ultra => 4,
    };
    let mut unread_in_breps: BTreeMap<String, usize> = BTreeMap::new();
    for (i, r) in records.iter().enumerate() {
        let b = &mut built[i];
        match &r.geometry {
            Geometry::Mesh(m) => b.mesh = m.clone(),
            Geometry::Brep(brep) => {
                for f in &brep.faces {
                    if let Some(m) = &f.mesh {
                        append(&mut b.mesh, m);
                    }
                }
                for u in &brep.unread {
                    *unread_in_breps.entry(u.clone()).or_default() += 1;
                }
                let faces_missing = brep.faces.iter().filter(|f| f.mesh.is_none() && f.surface.is_none()).count();
                failed += faces_missing;
                if !brep.edges.is_empty() {
                    b.edges.clear();
                    let tolerance = tolerance_for(i);
                    for (curve, from, to) in &brep.edges {
                        sample(curve, *from, *to, tolerance, &mut b.edges);
                    }
                }
            }
            Geometry::Extrusion(e) if !e.meshes.is_empty() => {
                if let Some(m) = e.meshes.iter().max_by_key(|m| m.triangles.len()) {
                    b.mesh = m.clone();
                }
            }
            Geometry::SubD(cage) => {
                // Four times the faces each round: keep it under two million.
                let mut l = levels;
                while l > 1 && cage.faces.len().saturating_mul(4usize.pow(l as u32)) > 2_000_000 {
                    l -= 1;
                }
                b.mesh = subd::surface(cage, l);
            }
            _ => {}
        }
    }

    // The tree: the model's node, turned Z up to Y up; a group for each
    // layer that holds something; each object under its layer.
    let mut scene = RawScene::default();
    if let Some((_, known)) = unit {
        scene.source_unit = Some(known);
    }
    let mut root = RawNode::group(name);
    root.transform = Transform::rotation(Vec3::X, -std::f32::consts::FRAC_PI_2);
    if !unit_name.is_empty() {
        root.metadata.insert("Units".into(), unit_name.clone());
    }
    if model_tolerance > 0.0 {
        root.metadata.insert("Absolute tolerance".into(), format!("{model_tolerance}"));
    }
    scene.nodes.push(root);
    scene.roots.push(0);

    let paths = layer_paths(&layers);
    let shown = layer_shown(&layers);
    let mut context = Context {
        scene,
        layers: &layers,
        paths: &paths,
        shown: &shown,
        materials: &materials,
        definitions: &definitions,
        records: &records,
        by_id: &by_id,
        built: &built,
        scale,
        layer_groups: HashMap::new(),
        rhino_materials: HashMap::new(),
        colour_materials: HashMap::new(),
        meshes: HashMap::new(),
        linked: BTreeMap::new(),
        missing: 0,
    };
    for (i, r) in records.iter().enumerate() {
        if members.contains(&r.attributes.id) || r.attributes.mode == 3 {
            continue;
        }
        let parent = context.layer_group(r.attributes.layer);
        context.place(i, parent, None, 0, &mut Vec::new());
    }
    let Context { mut scene, linked, missing, .. } = context;

    // What was left out, said.
    if failed > 0 {
        warnings.push(format!("{failed} faces could not be divided and are missing"));
    }
    if mitered > 0 {
        warnings.push(format!("{mitered} extrusions have mitered ends, which are drawn square"));
    }
    if !unread_in_breps.is_empty() {
        let list: Vec<String> = unread_in_breps.iter().map(|(k, n)| format!("{k} in {n}")).collect();
        warnings.push(format!("polysurfaces use pieces Tequila does not read, and those faces are missing: {}", list.join(", ")));
    }
    for (name, n) in &linked {
        warnings.push(format!("the block `{name}` is linked from another file, which is not opened; its {n} placements are empty"));
    }
    if missing > 0 {
        warnings.push(format!("{missing} blocks place a definition the file does not have"));
    }
    if !skipped.is_empty() {
        let list: Vec<String> = skipped.iter().map(|(k, n)| format!("{k} ×{n}")).collect();
        warnings.push(format!("objects Tequila does not render were left out: {}", list.join(", ")));
    }
    if scene.meshes.is_empty() && warnings.is_empty() {
        warnings.push("the file holds nothing to render".into());
    }
    scene.warnings = warnings;
    if divided_any {
        scene.tessellation = Some(quality.label().to_string());
    }
    scene
}

/// Each layer's full name, `Parent::Child`, as Rhino writes one.
fn layer_paths(layers: &[Layer]) -> Vec<String> {
    let by_id: HashMap<Uuid, usize> = layers.iter().enumerate().filter(|(_, l)| l.id != NIL).map(|(i, l)| (l.id, i)).collect();
    (0..layers.len())
        .map(|i| {
            let mut names = vec![layers[i].name.clone()];
            let mut at = i;
            while let Some(&p) = by_id.get(&layers[at].parent) {
                if names.len() > 64 || p == i {
                    break;
                }
                names.push(layers[p].name.clone());
                at = p;
            }
            names.reverse();
            names.join("::")
        })
        .collect()
}

/// Whether each layer is shown: on, and every layer above it on.
fn layer_shown(layers: &[Layer]) -> Vec<bool> {
    let by_id: HashMap<Uuid, usize> = layers.iter().enumerate().filter(|(_, l)| l.id != NIL).map(|(i, l)| (l.id, i)).collect();
    (0..layers.len())
        .map(|i| {
            let (mut at, mut depth) = (i, 0);
            loop {
                if !layers[at].visible {
                    return false;
                }
                match by_id.get(&layers[at].parent) {
                    Some(&p) if depth < 64 => {
                        at = p;
                        depth += 1;
                    }
                    _ => return true,
                }
            }
        })
        .collect()
}

/// What an object resolved to wear: a scene material, and the display
/// colour its members by parent inherit.
#[derive(Clone, Copy)]
struct Look {
    material: Option<usize>,
    rhino_material: Option<usize>,
    colour: [u8; 4],
}

struct Context<'a> {
    scene: RawScene,
    layers: &'a [Layer],
    paths: &'a [String],
    shown: &'a [bool],
    materials: &'a [Option<material::RhinoMaterial>],
    definitions: &'a HashMap<Uuid, Definition>,
    records: &'a [Record],
    by_id: &'a HashMap<Uuid, usize>,
    built: &'a [Built],
    /// What the file's lengths are multiplied by to be in the unit given.
    scale: f64,
    layer_groups: HashMap<usize, usize>,
    rhino_materials: HashMap<usize, usize>,
    colour_materials: HashMap<(String, [u8; 3]), usize>,
    /// Each record's mesh by the material it wears there.
    meshes: HashMap<(usize, Option<usize>), Option<usize>>,
    linked: BTreeMap<String, usize>,
    missing: usize,
}

impl Context<'_> {
    /// The group a layer's objects go in, made with its parents' the first
    /// time it is wanted.
    fn layer_group(&mut self, layer: i32) -> usize {
        let Some(index) = usize::try_from(layer).ok().filter(|&i| i < self.layers.len()) else { return 0 };
        if let Some(&g) = self.layer_groups.get(&index) {
            return g;
        }
        let parent = self.layers.iter().position(|l| l.id != NIL && l.id == self.layers[index].parent).filter(|&p| p != index);
        // Parents first, so the tree lists them before their children.
        let parent_node = match parent {
            Some(p) if self.layer_groups.len() < 4096 => self.layer_group(p as i32),
            _ => 0,
        };
        let mut group = RawNode::group(self.layers[index].name.clone());
        group.parent = Some(parent_node);
        self.scene.nodes.push(group);
        let node = self.scene.nodes.len() - 1;
        self.layer_groups.insert(index, node);
        node
    }

    fn look(&mut self, a: &Attributes, parent: Option<Look>) -> Look {
        let layer = usize::try_from(a.layer).ok().and_then(|i| self.layers.get(i));
        let layer_colour = layer.map_or([0; 4], |l| l.colour);
        let index = match (a.material_source, parent) {
            (1, _) => a.material,
            (3, Some(p)) => p.rhino_material.map_or(-1, |m| m as i32),
            _ => layer.map_or(-1, |l| l.material),
        };
        let rhino = usize::try_from(index).ok().filter(|&i| self.materials.get(i).is_some_and(|m| m.is_some()));
        let (colour, from_layer) = match (a.colour_source, parent) {
            (1, _) => (a.colour, false),
            (2, _) => match rhino.and_then(|i| self.materials[i].as_ref()) {
                Some(m) => (m.diffuse, false),
                None => (layer_colour, true),
            },
            (3, Some(p)) => (p.colour, false),
            _ => (layer_colour, true),
        };
        if let Some(i) = rhino {
            let material = match self.rhino_materials.get(&i) {
                Some(&m) => m,
                None => {
                    let definition = material::definition(self.materials[i].as_ref().unwrap(), &mut self.scene.textures);
                    self.scene.materials.push(definition);
                    let m = self.scene.materials.len() - 1;
                    self.rhino_materials.insert(i, m);
                    m
                }
            };
            return Look { material: Some(material), rhino_material: Some(i), colour };
        }
        // Rhino's Default layer is black until someone colours it: no
        // colour anyone chose, and the default material rather than black.
        let unset = from_layer && colour[..3] == [0, 0, 0] && layer.is_some_and(|l| l.name == "Default");
        if unset {
            return Look { material: parent.and_then(|p| p.material), rhino_material: None, colour };
        }
        let name = match (from_layer, layer) {
            (true, Some(_)) => self.paths[a.layer as usize].clone(),
            _ => format!("#{:02X}{:02X}{:02X}", colour[0], colour[1], colour[2]),
        };
        let key = (name.clone(), [colour[0], colour[1], colour[2]]);
        let material = match self.colour_materials.get(&key) {
            Some(&m) => m,
            None => {
                self.scene.materials.push(material::colour_material(&name, colour));
                let m = self.scene.materials.len() - 1;
                self.colour_materials.insert(key, m);
                m
            }
        };
        Look { material: Some(material), rhino_material: None, colour }
    }

    /// Put record `i` under `parent`: a part, or a block's group and its
    /// members, recursively.
    fn place(&mut self, i: usize, parent: usize, inherited: Option<Look>, depth: usize, stack: &mut Vec<Uuid>) {
        let records = self.records;
        let r = &records[i];
        let a = &r.attributes;
        let look = self.look(a, inherited);
        let layer = usize::try_from(a.layer).ok().filter(|&l| l < self.layers.len());
        let hidden = !a.visible || a.mode == 1 || layer.is_some_and(|l| !self.shown[l]);
        let mut node = match &r.geometry {
            Geometry::Block { definition, xform } => {
                let Some(d) = self.definitions.get(definition) else {
                    self.missing += 1;
                    return;
                };
                if d.linked && d.members.is_empty() {
                    *self.linked.entry(d.name.clone()).or_default() += 1;
                    return;
                }
                if depth >= DEEPEST || stack.contains(definition) {
                    return;
                }
                let mut group = RawNode::group(if a.name.is_empty() { d.name.clone() } else { a.name.clone() });
                let mut m = Mat4::IDENTITY;
                for (row, source) in m.m.iter_mut().zip(xform) {
                    for (v, s) in row.iter_mut().zip(source) {
                        *v = *s as f32;
                    }
                }
                for row in m.m.iter_mut().take(3) {
                    row[3] = (row[3] as f64 * self.scale) as f32;
                }
                group.transform = Transform::from_matrix(m).unwrap_or(Transform::IDENTITY);
                group.metadata.insert("Block".into(), d.name.clone());
                group
            }
            _ => {
                let key = (i, look.material);
                let mesh = match self.meshes.get(&key) {
                    Some(m) => *m,
                    None => {
                        let m = self.raw_mesh(i, look.material);
                        self.meshes.insert(key, m);
                        m
                    }
                };
                let Some(mesh) = mesh else { return };
                let kind = match &r.geometry {
                    Geometry::Mesh(_) => "Mesh",
                    Geometry::Brep(b) if b.faces.len() == 1 => "Surface",
                    Geometry::Brep(_) => "Polysurface",
                    Geometry::Extrusion(_) => "Extrusion",
                    Geometry::SubD(_) => "SubD",
                    _ => "Surface",
                };
                RawNode::part(if a.name.is_empty() { kind.to_string() } else { a.name.clone() }, mesh)
            }
        };
        node.parent = Some(parent);
        node.hidden = hidden;
        if let Some(l) = layer {
            node.layers = vec![self.paths[l].clone()];
        }
        if a.id != NIL {
            node.metadata.insert("Rhino ID".into(), format_uuid(a.id));
        }
        self.scene.nodes.push(node);
        let me = self.scene.nodes.len() - 1;
        if let Geometry::Block { definition, .. } = &r.geometry {
            let members = self.definitions[definition].members.clone();
            stack.push(*definition);
            for id in members {
                if let Some(&m) = self.by_id.get(&id) {
                    self.place(m, me, Some(look), depth + 1, stack);
                }
            }
            stack.pop();
        }
    }

    /// Record `i`'s triangles as a mesh of the scene, wearing `material`;
    /// `None` when it has none.
    fn raw_mesh(&mut self, i: usize, material: Option<usize>) -> Option<usize> {
        let b = &self.built[i];
        if b.mesh.triangles.is_empty() {
            return None;
        }
        let k = self.scale;
        let to = |p: &[f64; 3]| vec3((p[0] * k) as f32, (p[1] * k) as f32, (p[2] * k) as f32);
        let m = &b.mesh;
        let n = m.positions.len();
        let mut raw = RawMesh {
            positions: m.positions.iter().map(to).collect(),
            indices: m.triangles.iter().flatten().copied().collect(),
            material,
            name: self.records[i].attributes.name.clone(),
            edge_lines: b.edges.iter().map(to).collect(),
            ..Default::default()
        };
        if m.normals.len() == n && m.normals.iter().all(|v| v.iter().all(|c| c.is_finite())) {
            raw.normals = m.normals.iter().map(|v| vec3(v[0], v[1], v[2])).collect();
        }
        // Rhino's texture coordinates run up the image, as OpenGL's do;
        // the renderer's run down it.
        if m.uvs.len() == n && m.uvs.iter().all(|v| v.iter().all(|c| c.is_finite())) {
            raw.uvs = m.uvs.iter().map(|v| vec2(v[0], 1.0 - v[1])).collect();
        }
        if m.colors.len() == n {
            raw.colors = m
                .colors
                .iter()
                .map(|c| {
                    let s = tequila_core::spectrum::Spectrum::from_srgb_encoded(vec3(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0));
                    let l = s.to_linear_srgb();
                    vec4(l.x, l.y, l.z, 1.0)
                })
                .collect();
        }
        self.scene.meshes.push(raw);
        Some(self.scene.meshes.len() - 1)
    }
}

/// A saved render mesh added to an object's triangles.
fn append(to: &mut MeshData, m: &MeshData) {
    let base = to.positions.len() as u32;
    let n = m.positions.len();
    // A channel only one of the pieces has is dropped from all: a mesh
    // has a channel for all its vertices or none.
    let keep_normals = (to.positions.is_empty() || to.normals.len() == to.positions.len()) && m.normals.len() == n;
    let keep_uvs = (to.positions.is_empty() || to.uvs.len() == to.positions.len()) && m.uvs.len() == n;
    to.positions.extend_from_slice(&m.positions);
    if keep_normals {
        to.normals.extend_from_slice(&m.normals);
    } else {
        to.normals.clear();
    }
    if keep_uvs {
        to.uvs.extend_from_slice(&m.uvs);
    } else {
        to.uvs.clear();
    }
    to.triangles.extend(m.triangles.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
}

/// A face as the IGES divider takes it.
fn parametric(job: &Job) -> crate::iges::ParametricFace {
    type TrimFn = Arc<dyn Fn(f64) -> [f64; 2] + Send + Sync>;
    let s = job.surface.clone();
    let loops = job
        .loops
        .iter()
        .map(|l| {
            l.iter()
                .flat_map(|trim| {
                    let curve = trim.curve.clone();
                    trim.curve.pieces(trim.from, trim.to).into_iter().map(move |(a, b, min)| {
                        let c = curve.clone();
                        let f: TrimFn = Arc::new(move |t| {
                            let p = c.at(t);
                            [p[0], p[1]]
                        });
                        (f, a, b, min)
                    })
                })
                .collect()
        })
        .collect();
    crate::iges::ParametricFace { at: Arc::new(move |u, v| s.at(u, v)), domain: job.surface.domain(), spans: job.surface.spans(), loops, reversed: job.reversed }
}

/// Points along a curve from `from` to `to`, as segment end pairs, no
/// chord further than `tolerance` from it.
fn sample(curve: &Curve, from: f64, to: f64, tolerance: f64, out: &mut Vec<[f64; 3]>) {
    fn refine(curve: &Curve, a: f64, b: f64, pa: [f64; 3], pb: [f64; 3], tolerance: f64, depth: usize, out: &mut Vec<[f64; 3]>) {
        let m = (a + b) / 2.0;
        let pm = curve.at(m);
        let mid = [(pa[0] + pb[0]) / 2.0, (pa[1] + pb[1]) / 2.0, (pa[2] + pb[2]) / 2.0];
        let off = ((pm[0] - mid[0]).powi(2) + (pm[1] - mid[1]).powi(2) + (pm[2] - mid[2]).powi(2)).sqrt();
        if depth < 12 && off > tolerance {
            refine(curve, a, m, pa, pm, tolerance, depth + 1, out);
            refine(curve, m, b, pm, pb, tolerance, depth + 1, out);
        } else {
            out.push(pa);
            out.push(pb);
        }
    }
    let total = {
        let (p, q) = (curve.at(from), curve.at(to));
        ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
    };
    let start = out.len();
    for (a, b, min) in curve.pieces(from, to) {
        let n = min.max(1);
        for k in 0..n {
            let (s, t) = (a + (b - a) * k as f64 / n as f64, a + (b - a) * (k + 1) as f64 / n as f64);
            refine(curve, s, t, curve.at(s), curve.at(t), tolerance, 0, out);
        }
    }
    // A point, not an edge: a pole's.
    let length: f64 = out[start..].chunks_exact(2).map(|w| ((w[0][0] - w[1][0]).powi(2) + (w[0][1] - w[1][1]).powi(2) + (w[0][2] - w[1][2]).powi(2)).sqrt()).sum();
    if length <= tolerance && total <= tolerance {
        out.truncate(start);
    }
}

fn format_uuid(b: Uuid) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}
