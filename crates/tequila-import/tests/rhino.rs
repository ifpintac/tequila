//! Rhino .3dm import.
//!
//! No Rhino runs here, so most fixtures are written by the small writer
//! below, from the openNURBS field order the reader follows: they prove the
//! reader reads what it was written to read, not that Rhino writes it so.
//! What stands on firmer ground: `real_files` reads every `.3dm` in the
//! folder `TEQUILA_3DM_SAMPLES` names - files written by openNURBS itself
//! (McNeel's rhino3dm) or by another implementation - and is run by hand
//! (`cargo test -p tequila-import --test rhino -- --ignored`).

use flate2::write::ZlibEncoder;
use std::io::Write;
use tequila_import::raw::RawScene;
use tequila_import::step::Tessellation;
use tequila_import::{build, ImportOptions};
use tequila_scene::tree::NodeKind;
use tequila_scene::{CommandBus, SceneDocument};

// A writer of just enough 3dm.

const ANONYMOUS: u32 = 0x4000_8000;

struct Archive {
    version: u32,
}

fn crc(bytes: &[u8]) -> u32 {
    let mut c = flate2::Crc::new();
    c.update(bytes);
    c.sum()
}

impl Archive {
    fn chunk(&self, tcode: u32, body: &[u8]) -> Vec<u8> {
        let mut out = tcode.to_le_bytes().to_vec();
        let mut body = body.to_vec();
        if tcode & 0x8000 != 0 {
            let c = crc(&body);
            body.extend_from_slice(&c.to_le_bytes());
        }
        if self.version >= 50 {
            out.extend_from_slice(&(body.len() as i64).to_le_bytes());
        } else {
            out.extend_from_slice(&(body.len() as i32).to_le_bytes());
        }
        out.extend(body);
        out
    }

    fn short(&self, tcode: u32, value: i64) -> Vec<u8> {
        let mut out = tcode.to_le_bytes().to_vec();
        if self.version >= 50 {
            out.extend_from_slice(&value.to_le_bytes());
        } else {
            out.extend_from_slice(&(value as i32).to_le_bytes());
        }
        out
    }

    /// `BeginWrite3dmChunk(TCODE_ANONYMOUS_CHUNK, major, minor)`.
    fn anonymous(&self, major: i32, minor: i32, body: &[u8]) -> Vec<u8> {
        let mut b = major.to_le_bytes().to_vec();
        b.extend_from_slice(&minor.to_le_bytes());
        b.extend_from_slice(body);
        self.chunk(ANONYMOUS, &b)
    }

    /// `WriteObject`: the class wrapper.
    fn object(&self, class: &str, data: &[u8], userdata: &[Vec<u8>]) -> Vec<u8> {
        let mut inner = self.chunk(0x0002_FFFB, &uuid(class));
        inner.extend(self.chunk(0x0002_FFFC, data));
        for u in userdata {
            inner.extend_from_slice(u);
        }
        inner.extend(self.short(0x8002_7FFF, 0));
        self.chunk(0x0002_7FFA, &inner)
    }

    /// User data of version 2: a header chunk, then the payload.
    fn userdata(&self, class: &str, payload: &[u8]) -> Vec<u8> {
        let mut header = uuid(class).to_vec();
        header.extend_from_slice(&uuid(class));
        header.extend_from_slice(&0i32.to_le_bytes());
        for i in 0..16 {
            header.extend_from_slice(&(if i % 5 == 0 { 1.0f64 } else { 0.0 }).to_le_bytes());
        }
        let mut body = vec![0x20];
        body.extend(self.chunk(0x0002_FFF9, &header));
        body.extend(self.chunk(ANONYMOUS, payload));
        self.chunk(0x0002_7FFD, &body)
    }

    fn table(&self, tcode: u32, records: &[Vec<u8>]) -> Vec<u8> {
        let mut body: Vec<u8> = records.concat();
        body.extend(self.short(0xFFFF_FFFF, 0));
        self.chunk(tcode, &body)
    }

    /// `WriteCompressedBuffer`, deflated.
    fn compressed(&self, bytes: &[u8]) -> Vec<u8> {
        let mut out = (bytes.len() as u32).to_le_bytes().to_vec();
        if bytes.is_empty() {
            return out;
        }
        out.extend_from_slice(&crc(bytes).to_le_bytes());
        out.push(1);
        let mut z = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(bytes).unwrap();
        out.extend(self.chunk(ANONYMOUS, &z.finish().unwrap()));
        out
    }

    /// A whole file of tables: properties (with the openNURBS version),
    /// settings (with the unit), then the rest, then the end.
    fn file(&self, unit: i32, tables: &[Vec<u8>]) -> Vec<u8> {
        let mut out = format!("3D Geometry File Format {:>8}", self.version).into_bytes();
        out.extend(self.chunk(1, b" Tequila's test writer\n\x1a\0"));
        // A Rhino 7 openNURBS, or for the 4-byte archives one of 2006.
        let mut properties = self.short(0xA000_0026, if self.version >= 50 { 2_425_133_316 } else { 200_612_060 });
        properties.extend(self.short(0xFFFF_FFFF, 0));
        out.extend(self.chunk(0x1000_0014, &properties));
        let mut units = 102i32.to_le_bytes().to_vec();
        units.extend_from_slice(&unit.to_le_bytes());
        for v in [0.001f64, 0.0174, 0.01] {
            units.extend_from_slice(&v.to_le_bytes());
        }
        units.extend_from_slice(&0i32.to_le_bytes());
        units.extend_from_slice(&3i32.to_le_bytes());
        units.extend_from_slice(&0.001f64.to_le_bytes());
        units.extend_from_slice(&0u32.to_le_bytes());
        let mut settings = self.chunk(0x2000_8031, &units);
        settings.extend(self.short(0xFFFF_FFFF, 0));
        out.extend(self.chunk(0x1000_0015, &settings));
        for t in tables {
            out.extend_from_slice(t);
        }
        out.extend(self.chunk(0x7FFF, &0u64.to_le_bytes()));
        out
    }
}

/// A UUID's 16 bytes in the order the file stores them.
fn uuid(text: &str) -> [u8; 16] {
    let hex: Vec<u8> = text.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>().as_bytes().chunks(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect();
    let b = hex;
    [b[3], b[2], b[1], b[0], b[5], b[4], b[7], b[6], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]
}

fn string(s: &str) -> Vec<u8> {
    let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    let mut out = (units.len() as u32).to_le_bytes().to_vec();
    for u in units {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

fn f64s(values: &[f64]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn i32s(values: &[i32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

const MESH: &str = "4ED7D4E4-E947-11d3-BFE5-0010830122F0";
const BREP: &str = "60B5DBC5-E660-11d3-BFE4-0010830122F0";
const EXTRUSION: &str = "36F53175-72B8-4d47-BF1F-B4E6FC24F4B9";
const SUBD: &str = "F09BA4D9-455B-42C3-BA3B-E6CCACEF853B";
const LAYER: &str = "95809813-E985-11d3-BFE5-0010830122F0";
const MATERIAL: &str = "60B5DBBC-E660-11d3-BFE4-0010830122F0";
const IDEF: &str = "26F8BFF6-2618-417f-A158-153D64A94989";
const IREF: &str = "F9CFB638-B9D4-4340-87E3-C56E7865D96A";
const NURBS_CURVE: &str = "4ED7D4DD-E947-11d3-BFE5-0010830122F0";
const NURBS_SURFACE: &str = "4ED7D4DE-E947-11d3-BFE5-0010830122F0";
const PLANE_SURFACE: &str = "4ED7D4DF-E947-11d3-BFE5-0010830122F0";
const REV_SURFACE: &str = "A16220D3-163B-11d4-8000-0010830122F0";
const LINE_CURVE: &str = "4ED7D4DB-E947-11d3-BFE5-0010830122F0";
const ARC_CURVE: &str = "CF33BE2A-09B4-11d4-BFFB-0010830122F0";
const PBR: &str = "5694E1AC-40E6-44F4-9CA9-3B6D0E8C4440";
const TEXTURE: &str = "D6FF106D-329B-4F29-97E2-FD282A618020";

/// `ON_Layer`, version 1.6.
fn layer(a: &Archive, name: &str, colour: [u8; 4], visible: bool, material: i32, id: &str, parent: &str) -> Vec<u8> {
    // Its index is where it is in the table, which the ids here count.
    let index = i32::from_str_radix(&id[id.len() - 4..], 16).unwrap() - 1;
    let mut d = vec![0x16];
    d.extend(i32s(&[0, index, -1, material, 0]));
    d.extend_from_slice(&colour);
    d.extend_from_slice(&[0; 4]);
    d.extend(f64s(&[0.0, 1.0]));
    d.extend(string(name));
    d.push(visible as u8);
    d.extend(i32s(&[-1]));
    d.extend_from_slice(&[0, 0, 0, 255]);
    d.extend(f64s(&[0.0]));
    d.push(0);
    d.extend_from_slice(&uuid(id));
    d.extend_from_slice(&uuid(parent));
    d.push(1);
    a.chunk(0x2000_8050, &a.object(LAYER, &d, &[]))
}

/// A Rhino 6 material: an anonymous chunk with the component's attributes
/// and the basic fields, PBR user data when given.
fn material(a: &Archive, name: &str, diffuse: [u8; 4], shine: f64, transparency: f64, texture: Option<&str>, pbr: Option<(f64, f64)>) -> Vec<u8> {
    let mut component = i32s(&[1, 0]);
    component.extend_from_slice(&[0, 1]);
    component.extend_from_slice(&uuid("00000000-0000-0000-0000-00000000aaaa"));
    component.extend_from_slice(&[0, 0, 1]);
    component.extend_from_slice(&string(name));
    let mut body = a.chunk(0x4000_8002, &component);
    body.extend_from_slice(&[0; 16]);
    for c in [[0, 0, 0, 0], diffuse, [0, 0, 0, 0], [255, 255, 255, 0], [255, 255, 255, 0], diffuse] {
        body.extend_from_slice(&c);
    }
    body.extend(f64s(&[1.0, 0.0, shine, transparency]));
    let mut textures = i32s(&[texture.is_some() as i32]);
    if let Some(path) = texture {
        let mut t = uuid("00000000-0000-0000-0000-00000000bbbb").to_vec();
        t.extend_from_slice(&0u32.to_le_bytes());
        t.extend(string(path));
        t.push(1);
        t.extend_from_slice(&1u32.to_le_bytes());
        // Mode, filters, wrap, transform, colours, the transparency
        // texture's id, bump scale, and the blend: 272 bytes.
        t.extend_from_slice(&[0; 272]);
        textures.extend(a.object(TEXTURE, &a.anonymous(1, 0, &t), &[]));
    }
    body.extend(a.anonymous(1, 0, &textures));
    body.extend(i32s(&[0]));
    body.extend_from_slice(&[1, 0]);
    body.push(1);
    body.extend(f64s(&[1.0, 1.0, 1.5]));
    // The RDK material's id, and alpha transparency.
    body.extend_from_slice(&[0; 16]);
    body.push(0);
    let userdata: Vec<Vec<u8>> = pbr
        .map(|(metallic, roughness)| {
            let mut p = Vec::new();
            let colour = |p: &mut Vec<u8>, c: [f32; 4]| c.iter().for_each(|v| p.extend_from_slice(&v.to_le_bytes()));
            colour(&mut p, [0.8, 0.8, 0.8, 1.0]);
            p.extend(i32s(&[0]));
            p.extend(f64s(&[0.0]));
            colour(&mut p, [1.0, 1.0, 1.0, 1.0]);
            p.extend(f64s(&[0.0, metallic, 0.5, 0.0, roughness, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.52, 1.0, 0.0]));
            colour(&mut p, [0.0, 0.0, 0.0, 1.0]);
            p.extend(f64s(&[1.0]));
            vec![a.userdata(PBR, &a.anonymous(1, 2, &p))]
        })
        .unwrap_or_default();
    a.chunk(0x2000_8040, &a.object(MATERIAL, &a.anonymous(1, 0, &body), &userdata))
}

/// Tagged object attributes (version 2): id, layer, then numbered items;
/// before archive 50, the fixed list (version 1.2) with the items put in
/// their places.
fn attributes(a: &Archive, id: &str, layer: i32, name: &str, extra: &[(u8, Vec<u8>)]) -> Vec<u8> {
    if a.version < 50 {
        let item = |n: u8| extra.iter().find(|(i, _)| *i == n).map(|(_, v)| v.clone());
        let mut d = vec![0x12];
        d.extend_from_slice(&uuid(id));
        d.extend(i32s(&[layer]));
        d.extend(item(4).unwrap_or(i32s(&[-1])));
        d.extend(item(6).unwrap_or(vec![0; 4]));
        d.extend_from_slice(&[0; 4]);
        d.extend(f64s(&[0.0, 1.0]));
        d.extend(i32s(&[1]));
        d.extend(item(12).unwrap_or(vec![0]));
        d.extend(item(13).unwrap_or(vec![0]));
        d.push(0);
        d.extend(item(16).unwrap_or(vec![0]));
        d.extend(string(name));
        d.extend(string(""));
        d.extend(i32s(&[0]));
        d.extend(item(11).unwrap_or(vec![1]));
        return a.chunk(0x0200_8072, &d);
    }
    let mut d = vec![0x2D];
    d.extend_from_slice(&uuid(id));
    d.extend(i32s(&[layer]));
    if !name.is_empty() {
        d.push(1);
        d.extend(string(name));
    }
    for (item, value) in extra {
        d.push(*item);
        d.extend_from_slice(value);
    }
    d.push(0);
    a.chunk(0x0200_8072, &d)
}

fn record(a: &Archive, kind: i64, object: Vec<u8>, attributes: Vec<u8>) -> Vec<u8> {
    let mut body = a.short(0x8200_0071, kind);
    body.extend(object);
    body.extend(attributes);
    body.extend(a.short(0x8200_007F, 0));
    a.chunk(0x2000_8070, &body)
}

/// `ON_Mesh`, version 3.2 (compressed arrays) or 1.2 (as they are).
fn mesh(a: &Archive, compressed: bool, positions: &[[f32; 3]], quads: &[[u32; 4]], normals: bool) -> Vec<u8> {
    let mut d = vec![if compressed { 0x32 } else { 0x12 }];
    d.extend(i32s(&[positions.len() as i32, quads.len() as i32]));
    d.extend(f64s(&[0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0]));
    d.extend_from_slice(&[0; 64]);
    d.extend(i32s(&[0]));
    d.extend_from_slice(&[0; 5]);
    d.extend(i32s(&[4]));
    for q in quads {
        for i in q {
            d.extend_from_slice(&i.to_le_bytes());
        }
    }
    let v: Vec<u8> = positions.iter().flatten().flat_map(|c| c.to_le_bytes()).collect();
    let n: Vec<u8> = if normals {
        positions
            .iter()
            .flat_map(|p| {
                let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-9);
                [p[0] / l, p[1] / l, p[2] / l]
            })
            .flat_map(|c| c.to_le_bytes())
            .collect()
    } else {
        Vec::new()
    };
    if compressed {
        d.extend(a.compressed(&v));
        d.extend(a.compressed(&n));
        for _ in 0..3 {
            d.extend(a.compressed(&[]));
        }
    } else {
        d.extend(i32s(&[positions.len() as i32]));
        d.extend_from_slice(&v);
        d.extend(i32s(&[if normals { positions.len() as i32 } else { 0 }]));
        d.extend_from_slice(&n);
        d.extend(i32s(&[0, 0, 0]));
    }
    d.extend(i32s(&[0]));
    a.object(MESH, &d, &[])
}

fn cube_mesh(a: &Archive, compressed: bool, at: [f32; 3], size: f32) -> Vec<u8> {
    let p: Vec<[f32; 3]> = (0..8).map(|i| [at[0] + size * (i & 1) as f32, at[1] + size * ((i >> 1) & 1) as f32, at[2] + size * ((i >> 2) & 1) as f32]).collect();
    let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    mesh(a, compressed, &p, &quads, false)
}

// B-rep pieces.

fn nurbs_curve(a: &Archive, dim: i32, degree: i32, knots: &[f64], points: &[Vec<f64>], rational: bool) -> Vec<u8> {
    let mut d = vec![0x10];
    d.extend(i32s(&[dim, rational as i32, degree + 1, points.len() as i32, 0, 0]));
    d.extend_from_slice(&[0; 48]);
    d.extend(i32s(&[knots.len() as i32]));
    d.extend(f64s(knots));
    d.extend(i32s(&[points.len() as i32]));
    for p in points {
        d.extend(f64s(p));
    }
    a.object(NURBS_CURVE, &d, &[])
}

fn line_curve(a: &Archive, from: [f64; 3], to: [f64; 3], dim: i32) -> Vec<u8> {
    let mut d = vec![0x10];
    d.extend(f64s(&from));
    d.extend(f64s(&to));
    d.extend(f64s(&[0.0, 1.0]));
    d.extend(i32s(&[dim]));
    a.object(LINE_CURVE, &d, &[])
}

fn plane_surface(a: &Archive, origin: [f64; 3], x: [f64; 3], y: [f64; 3], size: [f64; 2]) -> Vec<u8> {
    let z = [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
    let mut d = vec![0x11];
    for v in [origin, x, y, z] {
        d.extend(f64s(&v));
    }
    d.extend(f64s(&[z[0], z[1], z[2], -(z[0] * origin[0] + z[1] * origin[1] + z[2] * origin[2])]));
    d.extend(f64s(&[0.0, size[0], 0.0, size[1], 0.0, size[0], 0.0, size[1]]));
    a.object(PLANE_SURFACE, &d, &[])
}

fn array(a: &Archive, count: usize, body: &[u8]) -> Vec<u8> {
    let mut b = vec![0x10];
    b.extend(i32s(&[count as i32]));
    b.extend_from_slice(body);
    a.chunk(ANONYMOUS, &b)
}

/// A B-rep of faces each with one outer loop of trims. `faces` holds each
/// face's surface (an object) and its trims as 2D curve objects; edges are
/// left without 3D curves, which only costs the model edges.
fn brep(a: &Archive, faces: &[(Vec<u8>, Vec<Vec<u8>>, bool)], render: Option<&[Vec<u8>]>) -> Vec<u8> {
    let mut c2 = Vec::new();
    let mut surfaces = Vec::new();
    let mut trims = Vec::new();
    let mut loops = Vec::new();
    let mut face_records = Vec::new();
    let mut c2_count = 0;
    let mut t_count = 0;
    for (fi, (surface, curves, reversed)) in faces.iter().enumerate() {
        surfaces.extend(i32s(&[1]));
        surfaces.extend_from_slice(surface);
        let mut ti = Vec::new();
        for curve in curves {
            c2.extend(i32s(&[1]));
            c2.extend_from_slice(curve);
            // Index, curve, its domain, edge, vertices, reversed, type,
            // iso, loop, tolerances; then the domain again, flags and
            // padding; then legacy tolerances.
            let mut t = i32s(&[t_count, c2_count]);
            t.extend(f64s(&[0.0, 1.0]));
            t.extend(i32s(&[-1, -1, -1, 0, 1, 0, fi as i32]));
            t.extend(f64s(&[0.0, 0.0, 0.0, 1.0]));
            t.extend_from_slice(&[0; 32]);
            t.extend(f64s(&[0.0, 0.0]));
            trims.extend(t);
            ti.push(t_count);
            c2_count += 1;
            t_count += 1;
        }
        let mut l = i32s(&[fi as i32, ti.len() as i32]);
        l.extend(i32s(&ti));
        l.extend(i32s(&[1, fi as i32]));
        loops.extend(l);
        let mut f = i32s(&[fi as i32, 1, fi as i32, fi as i32, *reversed as i32, 0]);
        f.truncate(4 * 6);
        face_records.extend(f);
    }
    let n = faces.len();
    let mut d = vec![0x32];
    d.extend(array(a, c2_count as usize, &c2));
    d.extend(array(a, 0, &[]));
    d.extend(array(a, n, &surfaces));
    d.extend(array(a, 0, &[]));
    d.extend(array(a, 0, &[]));
    d.extend(array(a, t_count as usize, &trims));
    d.extend(array(a, n, &loops));
    let mut faces_body = vec![0x10];
    faces_body.extend(i32s(&[n as i32]));
    faces_body.extend(face_records);
    d.extend(a.chunk(ANONYMOUS, &faces_body));
    d.extend_from_slice(&[0; 48]);
    let mut meshes = Vec::new();
    for i in 0..n {
        match render.and_then(|r| r.get(i)) {
            Some(m) => {
                meshes.push(1);
                meshes.extend_from_slice(m);
            }
            None => meshes.push(0),
        }
    }
    d.extend(a.chunk(ANONYMOUS, &meshes));
    d.extend(a.chunk(ANONYMOUS, &vec![0; n]));
    d.extend(i32s(&[0]));
    a.object(BREP, &d, &[])
}

/// A unit square's boundary in a face's parameters, from (0, 0) round.
fn square_trims(a: &Archive, size: [f64; 2]) -> Vec<Vec<u8>> {
    let c = [[0.0, 0.0], [size[0], 0.0], [size[0], size[1]], [0.0, size[1]]];
    (0..4).map(|i| line_curve(a, [c[i][0], c[i][1], 0.0], [c[(i + 1) % 4][0], c[(i + 1) % 4][1], 0.0], 2)).collect()
}

/// A box from six plane surfaces, its faces turned out.
fn box_brep(a: &Archive, size: f64) -> Vec<u8> {
    let s = size;
    let faces = [
        ([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
        ([0.0, 0.0, s], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, s, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([s, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
    ];
    let list: Vec<(Vec<u8>, Vec<Vec<u8>>, bool)> = faces.iter().map(|(o, x, y)| (plane_surface(a, *o, *x, *y, [s, s]), square_trims(a, [s, s]), false)).collect();
    brep(a, &list, None)
}

/// A fixture written where `TEQUILA_3DM_WRITE` says, when it says, for
/// another reader to check.
fn keep(name: &str, bytes: &[u8]) {
    if let Ok(folder) = std::env::var("TEQUILA_3DM_WRITE") {
        let _ = std::fs::create_dir_all(&folder);
        let _ = std::fs::write(std::path::Path::new(&folder).join(name), bytes);
    }
}

fn import(bytes: &[u8]) -> (SceneDocument, tequila_import::ImportReport) {
    let scene = tequila_import::rhino::read(bytes, "model").expect("reads");
    let mut document = SceneDocument::new();
    let (command, report) = build(&document, None, scene, &ImportOptions::default(), None).expect("builds");
    CommandBus::new().apply(&mut document, command).unwrap();
    document.validate().unwrap();
    (document, report)
}

fn parts(document: &SceneDocument) -> Vec<&tequila_scene::tree::SceneNode> {
    document.tree.iter().filter(|n| matches!(n.kind, NodeKind::Part { .. })).collect()
}

/// A mesh's signed volume: positive when its triangles face out.
fn volume(scene: &RawScene, mesh: usize) -> f64 {
    let m = &scene.meshes[mesh];
    m.indices
        .chunks_exact(3)
        .map(|t| {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| m.positions[i as usize]);
            let (a, b, c) = ([a.x as f64, a.y as f64, a.z as f64], [b.x as f64, b.y as f64, b.z as f64], [c.x as f64, c.y as f64, c.z as f64]);
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
        })
        .sum()
}

/// A 10 x 10 flat NURBS patch at `at`, its face cut by a circle of four
/// rational arcs: two loops on one face.
fn plate_object(a: &Archive, at: [f64; 3]) -> Vec<u8> {
    // A flat bicubic patch over 0..10 x 0..10, parameters 0..1.
    let mut d = vec![0x10];
    d.extend(i32s(&[3, 0, 4, 4, 4, 4, 0, 0]));
    d.extend_from_slice(&[0; 48]);
    for _ in 0..2 {
        d.extend(i32s(&[6]));
        d.extend(f64s(&[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]));
    }
    d.extend(i32s(&[16]));
    for i in 0..4 {
        for j in 0..4 {
            d.extend(f64s(&[at[0] + 10.0 * i as f64 / 3.0, at[1] + 10.0 * j as f64 / 3.0, at[2]]));
        }
    }
    let surface = a.object(NURBS_SURFACE, &d, &[]);
    // The circle round (0.5, 0.5) of radius 0.2, in parameters.
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let (cx, cy, r) = (0.5, 0.5, 0.2);
    let mut circle = Vec::new();
    for q in 0..4 {
        let a0 = q as f64 * std::f64::consts::FRAC_PI_2;
        let p = |a: f64, w: f64, s: f64| vec![(cx + s * r * a.cos()) * w, (cy + s * r * a.sin()) * w, w];
        circle.push(nurbs_curve(a, 2, 2, &[0.0, 0.0, 1.0, 1.0], &[p(a0, 1.0, 1.0), p(a0 + std::f64::consts::FRAC_PI_4, h, 1.0 / h), p(a0 + std::f64::consts::FRAC_PI_2, 1.0, 1.0)], true));
    }
    let outer = square_trims(a, [1.0, 1.0]);
    // Two loops on one face: written as a face with the outer loop, then
    // the hole's trims appended to a second loop by hand.
    let mut c2 = Vec::new();
    let mut trims = Vec::new();
    for (i, c) in outer.iter().chain(circle.iter()).enumerate() {
        c2.extend(i32s(&[1]));
        c2.extend_from_slice(c);
        let mut t = i32s(&[i as i32, i as i32]);
        t.extend(f64s(&[0.0, 1.0]));
        t.extend(i32s(&[-1, -1, -1, 0, 1, 0, if i < 4 { 0 } else { 1 }]));
        t.extend(f64s(&[0.0, 0.0, 0.0, 1.0]));
        t.extend_from_slice(&[0; 32]);
        t.extend(f64s(&[0.0, 0.0]));
        trims.extend(t);
    }
    let mut loops = i32s(&[0, 4, 0, 1, 2, 3, 1, 0]);
    loops.extend(i32s(&[1, 4, 4, 5, 6, 7, 2, 0]));
    let mut b = vec![0x32];
    b.extend(array(a, 8, &c2));
    b.extend(array(a, 0, &[]));
    let mut s = i32s(&[1]);
    s.extend(surface);
    b.extend(array(a, 1, &s));
    b.extend(array(a, 0, &[]));
    b.extend(array(a, 0, &[]));
    b.extend(array(a, 8, &trims));
    b.extend(array(a, 2, &loops));
    b.extend(array(a, 1, &i32s(&[0, 2, 0, 1, 0, 0, 0])));
    b.extend_from_slice(&[0; 48]);
    b.extend(a.chunk(ANONYMOUS, &[0]));
    b.extend(a.chunk(ANONYMOUS, &[0]));
    b.extend(i32s(&[0]));
    a.object(BREP, &b, &[])
}

/// A sphere as Rhino makes one, at `at`: a face on a surface of revolution
/// of a half circle, bounded by two seams and two poles.
fn ball_object(a: &Archive, r: f64, at: [f64; 3]) -> Vec<u8> {
    // The profile: a half circle in the XZ plane, -90° to 90°.
    let mut arc = vec![0x10];
    for v in [at, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]] {
        arc.extend(f64s(&v));
    }
    arc.extend(f64s(&[0.0, -1.0, 0.0, at[1]]));
    arc.extend(f64s(&[r]));
    arc.extend(f64s(&[at[0] + r, at[1], at[2], at[0], at[1], at[2] + r, at[0] - r, at[1], at[2]]));
    arc.extend(f64s(&[-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2, 0.0, 1.0]));
    arc.extend(i32s(&[3]));
    let profile = a.object(ARC_CURVE, &arc, &[]);
    let mut rev = vec![0x20];
    rev.extend(f64s(&[at[0], at[1], at[2], at[0], at[1], at[2] + 1.0]));
    rev.extend(f64s(&[0.0, std::f64::consts::TAU, 0.0, 1.0]));
    rev.extend_from_slice(&[0; 48]);
    rev.extend(i32s(&[0]));
    rev.push(1);
    rev.extend(profile);
    let surface = a.object(REV_SURFACE, &rev, &[]);
    let trims = square_trims(a, [1.0, 1.0]);
    brep(a, &[(surface, trims, false)], None)
}

/// A circle of radius `r` extruded `h` up from `at`, capped.
fn rod_object(a: &Archive, r: f64, h: f64, at: [f64; 3]) -> Vec<u8> {
    let mut arc = vec![0x10];
    for v in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
        arc.extend(f64s(&v));
    }
    arc.extend(f64s(&[0.0, 0.0, 1.0, 0.0]));
    arc.extend(f64s(&[r]));
    arc.extend(f64s(&[r, 0.0, 0.0, 0.0, r, 0.0, -r, 0.0, 0.0]));
    arc.extend(f64s(&[0.0, std::f64::consts::TAU, 0.0, 1.0]));
    arc.extend(i32s(&[2]));
    let mut e = a.object(ARC_CURVE, &arc, &[]);
    e.extend(f64s(&[at[0], at[1], at[2], at[0], at[1], at[2] + h]));
    e.extend(f64s(&[0.0, 1.0]));
    e.extend(f64s(&[0.0, 1.0, 0.0]));
    e.extend_from_slice(&[0, 0]);
    e.extend(f64s(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]));
    e.extend(f64s(&[0.0, h]));
    e.push(0);
    e.extend(i32s(&[1]));
    e.extend_from_slice(&[1, 1]);
    a.object(EXTRUSION, &a.anonymous(1, 2, &e), &[])
}

/// A SubD whose cage is a cube of `size` at `at`, as Rhino 7 writes one:
/// level zero's vertices, edges and faces, each ended by its additions.
fn subd_object(a: &Archive, size: f64, at: [f64; 3]) -> Vec<u8> {
    let points: Vec<[f64; 3]> = (0..8).map(|i| [at[0] + (i & 1) as f64 * size, at[1] + ((i >> 1) & 1) as f64 * size, at[2] + ((i >> 2) & 1) as f64 * size]).collect();
    let faces: [[usize; 4]; 6] = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    let mut edges: Vec<[usize; 2]> = Vec::new();
    for f in &faces {
        for j in 0..4 {
            let e = [f[j].min(f[(j + 1) % 4]), f[j].max(f[(j + 1) % 4])];
            if !edges.contains(&e) {
                edges.push(e);
            }
        }
    }
    let (v0, e0, f0) = (1u32, 1 + 8, 1 + 8 + edges.len() as u32);
    let end = 255u8;
    let base = |id: u32| {
        let mut b = id.to_le_bytes().to_vec();
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.push(end);
        b
    };
    let pointer = |id: u32, flag: u8| {
        let mut b = id.to_le_bytes().to_vec();
        b.push(flag);
        b
    };
    let mut level = 0u16.to_le_bytes().to_vec();
    level.extend_from_slice(&[4, 4, 4]);
    level.extend_from_slice(&[0; 48]);
    for p in [v0, e0, f0, f0 + 6] {
        level.extend_from_slice(&p.to_le_bytes());
    }
    for (i, p) in points.iter().enumerate() {
        level.extend(base(v0 + i as u32));
        level.push(1);
        level.extend(f64s(p));
        let incident: Vec<usize> = (0..edges.len()).filter(|&e| edges[e].contains(&i)).collect();
        let fs: Vec<usize> = (0..6).filter(|&f| faces[f].contains(&i)).collect();
        level.extend_from_slice(&(incident.len() as u16).to_le_bytes());
        level.extend_from_slice(&(fs.len() as u16).to_le_bytes());
        level.push(0);
        level.extend_from_slice(&(incident.len() as u16).to_le_bytes());
        for e in &incident {
            level.extend(pointer(e0 + *e as u32, 0));
        }
        level.extend_from_slice(&(fs.len() as u16).to_le_bytes());
        for f in &fs {
            level.extend(pointer(f0 + *f as u32, 0));
        }
        level.push(end);
    }
    for (i, e) in edges.iter().enumerate() {
        level.extend(base(e0 + i as u32));
        level.push(1);
        level.extend_from_slice(&2u16.to_le_bytes());
        level.extend(f64s(&[0.0, 0.0, 0.0]));
        level.extend_from_slice(&2u16.to_le_bytes());
        level.extend(pointer(v0 + e[0] as u32, 0));
        level.extend(pointer(v0 + e[1] as u32, 0));
        let fs: Vec<usize> = (0..6).filter(|&f| (0..4).any(|j| [faces[f][j].min(faces[f][(j + 1) % 4]), faces[f][j].max(faces[f][(j + 1) % 4])] == *e)).collect();
        level.extend_from_slice(&(fs.len() as u16).to_le_bytes());
        for f in fs {
            level.extend(pointer(f0 + f as u32, 0));
        }
        level.push(end);
    }
    for (i, f) in faces.iter().enumerate() {
        level.extend(base(f0 + i as u32));
        level.extend_from_slice(&[0; 8]);
        level.extend_from_slice(&4u16.to_le_bytes());
        level.extend_from_slice(&4u16.to_le_bytes());
        for j in 0..4 {
            let (p, q) = (f[j], f[(j + 1) % 4]);
            let e = edges.iter().position(|e| *e == [p.min(q), p.max(q)]).unwrap();
            level.extend(pointer(e0 + e as u32, (p > q) as u8));
        }
        level.push(end);
    }
    level.push(0);
    let mut subdimple = 1u32.to_le_bytes().to_vec();
    subdimple.extend_from_slice(&[0; 12]);
    subdimple.extend_from_slice(&[0; 48]);
    subdimple.extend(a.anonymous(1, 1, &level));
    let mut data = vec![1];
    data.extend(a.anonymous(1, 0, &subdimple));
    a.object(SUBD, &data, &[])
}

// The tests.

/// Every archive version Rhino 5 to 8 writes: a compressed mesh and a
/// plain one, named, on layers, the unit read.
#[test]
fn meshes_read_in_every_archive_version() {
    for version in [5, 50, 60, 70, 80] {
        let a = Archive { version };
        let layers = a.table(0x1000_0011, &[
            layer(&a, "Default", [0, 0, 0, 0], true, -1, "11111111-0000-0000-0000-000000000001", "00000000-0000-0000-0000-000000000000"),
            layer(&a, "Parts", [200, 30, 30, 0], true, -1, "11111111-0000-0000-0000-000000000002", "00000000-0000-0000-0000-000000000000"),
        ]);
        let objects = a.table(0x1000_0013, &[
            record(&a, 0x20, cube_mesh(&a, true, [0.0, 0.0, 0.0], 10.0), attributes(&a, "22222222-0000-0000-0000-000000000001", 1, "Squeezed", &[])),
            record(&a, 0x20, cube_mesh(&a, false, [20.0, 0.0, 0.0], 5.0), attributes(&a, "22222222-0000-0000-0000-000000000002", 0, "", &[])),
        ]);
        let bytes = a.file(2, &[layers, objects]);
        keep(&format!("meshes_v{version}.3dm"), &bytes);
        let scene = tequila_import::rhino::read(&bytes, "model").expect("reads");
        assert_eq!(scene.source_unit, Some(tequila_core::units::LengthUnit::Millimetre), "v{version}");
        assert_eq!(scene.meshes.len(), 2, "v{version}: {:?}", scene.warnings);
        assert!(scene.meshes.iter().all(|m| m.indices.len() == 36), "v{version}: six quads, twelve triangles each");
        let names: Vec<&str> = scene.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"Squeezed") && names.contains(&"Mesh") && names.contains(&"Parts"), "v{version}: {names:?}");
        let squeezed = scene.nodes.iter().find(|n| n.name == "Squeezed").unwrap();
        assert_eq!(squeezed.layers, vec!["Parts".to_string()]);
        // The Parts layer's red: a plastic named after the layer.
        let m = scene.meshes[squeezed.mesh.unwrap()].material.expect("a material");
        assert_eq!(scene.materials[m].name, "Parts");
        // On the Default layer, black and untouched: the default material.
        let plain = scene.nodes.iter().find(|n| n.name == "Mesh").unwrap();
        assert_eq!(scene.meshes[plain.mesh.unwrap()].material, None);
    }
}

/// The model is turned Z up to Y up on its top node, brought into metres
/// and set on the floor: a 10 mm cube standing on Rhino's ground stands on
/// Tequila's.
#[test]
fn rhino_z_up_comes_in_y_up_in_metres() {
    let a = Archive { version: 70 };
    let objects = a.table(0x1000_0013, &[record(&a, 0x20, cube_mesh(&a, true, [0.0, 0.0, 5.0], 10.0), attributes(&a, "22222222-0000-0000-0000-000000000001", 0, "Cube", &[]))]);
    let (document, report) = import(&a.file(2, &[objects]));
    let bounds = document.bounds();
    assert!((bounds.max.y - bounds.min.y - 0.01).abs() < 1e-6, "{bounds:?}");
    assert!(bounds.min.y.abs() < 1e-6, "on the floor: {bounds:?}");
    // Rhino's +Z is up, its -Y towards the viewer of its Front view.
    assert!((bounds.min.z + 0.01).abs() < 1e-6 && bounds.max.z.abs() < 1e-6, "{bounds:?}");
    assert!((report.lifted + 0.005).abs() < 1e-6, "lifted {}", report.lifted);
}

/// Layers: a sublayer's full name, a layer switched off hides what is on
/// it, a hidden object stays hidden, and an object's own colour makes its
/// own material.
#[test]
fn layers_colours_and_hidden_objects() {
    let a = Archive { version: 70 };
    let nil = "00000000-0000-0000-0000-000000000000";
    let layers = a.table(0x1000_0011, &[
        layer(&a, "Body", [10, 120, 200, 0], true, -1, "11111111-0000-0000-0000-000000000001", nil),
        layer(&a, "Buttons", [250, 200, 0, 0], true, -1, "11111111-0000-0000-0000-000000000002", "11111111-0000-0000-0000-000000000001"),
        layer(&a, "Construction", [90, 90, 90, 0], false, -1, "11111111-0000-0000-0000-000000000003", nil),
    ]);
    let objects = a.table(0x1000_0013, &[
        record(&a, 0x20, cube_mesh(&a, true, [0.0; 3], 1.0), attributes(&a, "22222222-0000-0000-0000-000000000001", 1, "Button", &[])),
        record(&a, 0x20, cube_mesh(&a, true, [2.0, 0.0, 0.0], 1.0), attributes(&a, "22222222-0000-0000-0000-000000000002", 2, "Guide", &[])),
        record(&a, 0x20, cube_mesh(&a, true, [4.0, 0.0, 0.0], 1.0), attributes(&a, "22222222-0000-0000-0000-000000000003", 0, "Secret", &[(11, vec![0])])),
        // By object: colour (6) then its source (13).
        record(&a, 0x20, cube_mesh(&a, true, [6.0, 0.0, 0.0], 1.0), attributes(&a, "22222222-0000-0000-0000-000000000004", 0, "Orange", &[(6, vec![255, 128, 0, 0]), (13, vec![1])])),
    ]);
    let bytes = a.file(2, &[layers, objects]);
    keep("layers.3dm", &bytes);
    let scene = tequila_import::rhino::read(&bytes, "model").unwrap();
    let node = |name: &str| scene.nodes.iter().find(|n| n.name == name).unwrap_or_else(|| panic!("{name}"));
    assert_eq!(node("Button").layers, vec!["Body::Buttons".to_string()]);
    assert_eq!(scene.nodes[node("Button").parent.unwrap()].name, "Buttons");
    assert_eq!(scene.nodes[scene.nodes[node("Button").parent.unwrap()].parent.unwrap()].name, "Body", "the layer tree is the scene tree");
    assert!(!node("Button").hidden);
    assert!(node("Guide").hidden, "on a layer that is off");
    assert!(node("Secret").hidden, "hidden itself");
    let orange = scene.meshes[node("Orange").mesh.unwrap()].material.unwrap();
    assert_eq!(scene.materials[orange].name, "#FF8000");
}

/// A Rhino 7 material with PBR settings becomes a metal of that roughness;
/// a basic one a plastic of its colour and shine, its colour map attached.
#[test]
fn materials_map_to_openpbr() {
    let a = Archive { version: 70 };
    let materials = a.table(0x1000_0010, &[
        material(&a, "Brushed steel", [180, 180, 180, 0], 0.0, 0.0, None, Some((1.0, 0.3))),
        material(&a, "Red plastic", [200, 0, 0, 0], 200.0, 0.0, Some("C:\\Textures\\wood.png"), None),
        material(&a, "Glass", [255, 255, 255, 0], 255.0, 0.9, None, None),
    ]);
    let objects = a.table(0x1000_0013, &[
        record(&a, 0x20, cube_mesh(&a, true, [0.0; 3], 1.0), attributes(&a, "22222222-0000-0000-0000-000000000001", 0, "Steel", &[(4, i32s(&[0])), (16, vec![1])])),
        record(&a, 0x20, cube_mesh(&a, true, [2.0, 0.0, 0.0], 1.0), attributes(&a, "22222222-0000-0000-0000-000000000002", 0, "Plastic", &[(4, i32s(&[1])), (16, vec![1])])),
        record(&a, 0x20, cube_mesh(&a, true, [4.0, 0.0, 0.0], 1.0), attributes(&a, "22222222-0000-0000-0000-000000000003", 0, "Pane", &[(4, i32s(&[2])), (16, vec![1])])),
    ]);
    let bytes = a.file(2, &[materials, objects]);
    keep("materials.3dm", &bytes);
    let scene = tequila_import::rhino::read(&bytes, "model").unwrap();
    let wears = |name: &str| {
        let node = scene.nodes.iter().find(|n| n.name == name).unwrap();
        &scene.materials[scene.meshes[node.mesh.unwrap()].material.unwrap()]
    };
    let steel = wears("Steel");
    assert_eq!(steel.name, "Brushed steel");
    assert_eq!(steel.material().base_metalness, 1.0);
    assert!((steel.material().specular_roughness - 0.3).abs() < 1e-6);
    let plastic = wears("Plastic");
    assert_eq!(plastic.material().base_metalness, 0.0);
    assert!((plastic.material().specular_roughness - (2.0f32 / 202.0).sqrt()).abs() < 1e-4);
    assert!(plastic.material().base_color.r > 5.0 * plastic.material().base_color.g.max(1e-3));
    assert_eq!(scene.textures.len(), 1);
    assert_eq!(scene.textures[0].uri, "C:\\Textures\\wood.png");
    let glass = wears("Pane");
    assert!((glass.material().transmission_weight - 0.9).abs() < 1e-6);
    assert!((glass.material().specular_ior - 1.5).abs() < 1e-6, "an IOR of 1 is read as glass's");
}

/// A box of six planes with no saved meshes is divided: closed, facing
/// out, its volume the box's.
#[test]
fn a_brep_without_meshes_is_divided() {
    let a = Archive { version: 70 };
    let objects = a.table(0x1000_0013, &[record(&a, 0x10, box_brep(&a, 10.0), attributes(&a, "22222222-0000-0000-0000-000000000001", 0, "Box", &[]))]);
    let bytes = a.file(2, &[objects]);
    keep("box_planes.3dm", &bytes);
    let scene = tequila_import::rhino::read(&bytes, "model").unwrap();
    assert_eq!(scene.meshes.len(), 1, "{:?}", scene.warnings);
    let v = volume(&scene, 0);
    println!("box of planes: volume {v:.4} of 1000, {} triangles", scene.meshes[0].indices.len() / 3);
    assert!((v - 1000.0).abs() < 1e-3, "volume {v}");
    assert_eq!(scene.tessellation.as_deref(), Some("Medium"));
    assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);
}

/// Saved render meshes are used as they are, and nothing is divided.
#[test]
fn a_brep_with_saved_meshes_uses_them() {
    let a = Archive { version: 60 };
    let render: Vec<Vec<u8>> = (0..1).map(|_| cube_mesh(&a, true, [0.0; 3], 3.0)).collect();
    let face = vec![(plane_surface(&a, [0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [3.0, 3.0]), square_trims(&a, [3.0, 3.0]), false)];
    let objects = a.table(0x1000_0013, &[record(&a, 0x10, brep(&a, &face, Some(&render)), attributes(&a, "22222222-0000-0000-0000-000000000001", 0, "Cached", &[]))]);
    let bytes = a.file(2, &[objects]);
    keep("cached_mesh.3dm", &bytes);
    let scene = tequila_import::rhino::read(&bytes, "model").unwrap();
    assert_eq!(scene.meshes[0].indices.len(), 36, "the saved cube, not the divided square");
    assert_eq!(scene.tessellation, None);
}

/// A NURBS surface face with a hole cut by a circle of four rational arcs:
/// the hole is open, and the area is the square's less the circle's.
#[test]
fn a_trimmed_nurbs_face_keeps_its_hole() {
    let a = Archive { version: 70 };
    let objects = a.table(0x1000_0013, &[record(&a, 0x10, plate_object(&a, [0.0; 3]), attributes(&a, "22222222-0000-0000-0000-000000000001", 0, "Plate", &[]))]);
    let bytes = a.file(2, &[objects]);
    keep("plate_hole.3dm", &bytes);
    let scene = tequila_import::rhino::read(&bytes, "model").unwrap();
    assert_eq!(scene.meshes.len(), 1, "{:?}", scene.warnings);
    let m = &scene.meshes[0];
    let area: f64 = m
        .indices
        .chunks_exact(3)
        .map(|t| {
            let [p, q, s] = [t[0], t[1], t[2]].map(|i| m.positions[i as usize]);
            let (u, v) = (q - p, s - p);
            (u.cross(v)).length() as f64 / 2.0
        })
        .sum();
    let expected = 100.0 - std::f64::consts::PI * 2.0 * 2.0;
    println!("plate with a hole: area {area:.3} of {expected:.3}, {} triangles", m.indices.len() / 3);
    assert!((area - expected).abs() < 0.05 * 4.0 * std::f64::consts::PI, "area {area}, expected {expected}");
    assert!(m.positions.iter().all(|p| ((p.x - 5.0).powi(2) + (p.y - 5.0).powi(2)).sqrt() >= 2.0 - 0.01), "nothing inside the hole");
}

/// A sphere as Rhino makes one: a face on a surface of revolution of a
/// half circle, bounded by two seams and two poles.
#[test]
fn a_revolved_face_is_round() {
    let a = Archive { version: 70 };
    let r = 3.0;
    let objects = a.table(0x1000_0013, &[record(&a, 0x10, ball_object(&a, r, [0.0; 3]), attributes(&a, "22222222-0000-0000-0000-000000000001", 0, "Ball", &[]))]);
    let bytes = a.file(2, &[objects]);
    keep("ball_revolved.3dm", &bytes);
    let scene = tequila_import::rhino::read(&bytes, "model").unwrap();
    assert_eq!(scene.meshes.len(), 1, "{:?}", scene.warnings);
    let m = &scene.meshes[0];
    for p in &m.positions {
        assert!((p.length() - r as f32).abs() < 1e-3, "{p:?} is off the sphere");
    }
    let v = volume(&scene, 0);
    let exact = 4.0 / 3.0 * std::f64::consts::PI * r * r * r;
    println!("revolved sphere: volume {v:.3} of {exact:.3}, {} triangles", m.indices.len() / 3);
    assert!(v > 0.0 && (v - exact).abs() < exact * 0.02, "volume {v}, a sphere's {exact}: faces out, closed");
}

/// An extrusion of a circle with caps: a closed cylinder.
#[test]
fn a_capped_extrusion_is_a_closed_cylinder() {
    let a = Archive { version: 70 };
    let r = 2.0;
    let objects = a.table(0x1000_0013, &[record(&a, 0x4000_0000, rod_object(&a, r, 5.0, [0.0; 3]), attributes(&a, "22222222-0000-0000-0000-000000000001", 0, "Rod", &[]))]);
    let bytes = a.file(2, &[objects]);
    keep("rod_extrusion.3dm", &bytes);
    let scene = tequila_import::rhino::read(&bytes, "model").unwrap();
    assert_eq!(scene.meshes.len(), 1, "{:?}", scene.warnings);
    let v = volume(&scene, 0);
    let exact = std::f64::consts::PI * r * r * 5.0;
    println!("capped extrusion: volume {v:.3} of {exact:.3}, {} triangles", scene.meshes[0].indices.len() / 3);
    assert!((v - exact).abs() < exact * 0.02, "volume {v}, the cylinder's {exact}");
}

/// A block defined once and placed three times shares one mesh; a block
/// in a block is placed through both.
#[test]
fn blocks_are_meshed_once_and_placed() {
    let a = Archive { version: 70 };
    let idef = |id: &str, name: &str, members: &[&str]| {
        let mut component = i32s(&[1, 0]);
        component.extend_from_slice(&[0, 1]);
        component.extend_from_slice(&uuid(id));
        component.extend_from_slice(&[0, 0, 1]);
        component.extend(string(name));
        let mut b = a.chunk(0x4000_8002, &component);
        b.extend_from_slice(&1u32.to_le_bytes());
        let mut unit = 2u32.to_le_bytes().to_vec();
        unit.extend(f64s(&[0.001]));
        unit.extend(string(""));
        b.extend(a.anonymous(1, 0, &unit));
        for _ in 0..3 {
            b.extend(string(""));
        }
        b.extend_from_slice(&[0; 48]);
        b.push(1);
        b.extend(i32s(&[members.len() as i32]));
        for m in members {
            b.extend_from_slice(&uuid(m));
        }
        b.push(0);
        a.chunk(0x2000_8076, &a.object(IDEF, &a.anonymous(1, 0, &b), &[]))
    };
    let iref = |definition: &str, x: f64| {
        let mut d = vec![0x10];
        d.extend_from_slice(&uuid(definition));
        d.extend(f64s(&[1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]));
        d.extend_from_slice(&[0; 48]);
        a.object(IREF, &d, &[])
    };
    let idefs = a.table(0x1000_0021, &[
        idef("33333333-0000-0000-0000-000000000001", "Screw", &["22222222-0000-0000-0000-0000000000a1"]),
        idef("33333333-0000-0000-0000-000000000002", "Pair", &["22222222-0000-0000-0000-0000000000b1", "22222222-0000-0000-0000-0000000000b2"]),
    ]);
    let member = |id: &str| {
        let mut d = vec![0x2D];
        d.extend_from_slice(&uuid(id));
        d.extend(i32s(&[0]));
        d.extend_from_slice(&[12, 3, 0]);
        a.chunk(0x0200_8072, &d)
    };
    let objects = a.table(0x1000_0013, &[
        record(&a, 0x20, cube_mesh(&a, true, [0.0; 3], 1.0), member("22222222-0000-0000-0000-0000000000a1")),
        record(&a, 0x1000, iref("33333333-0000-0000-0000-000000000001", 0.0), member("22222222-0000-0000-0000-0000000000b1")),
        record(&a, 0x1000, iref("33333333-0000-0000-0000-000000000001", 3.0), member("22222222-0000-0000-0000-0000000000b2")),
        record(&a, 0x1000, iref("33333333-0000-0000-0000-000000000001", 10.0), attributes(&a, "22222222-0000-0000-0000-000000000001", 0, "", &[])),
        record(&a, 0x1000, iref("33333333-0000-0000-0000-000000000001", 20.0), attributes(&a, "22222222-0000-0000-0000-000000000002", 0, "", &[])),
        record(&a, 0x1000, iref("33333333-0000-0000-0000-000000000002", 30.0), attributes(&a, "22222222-0000-0000-0000-000000000003", 0, "Two screws", &[])),
    ]);
    let bytes = a.file(2, &[idefs, objects]);
    keep("blocks.3dm", &bytes);
    let scene = tequila_import::rhino::read(&bytes, "model").unwrap();
    assert_eq!(scene.meshes.len(), 1, "one mesh for every screw: {:?}", scene.warnings);
    let (document, _) = import(&bytes);
    let screws = parts(&document);
    assert_eq!(screws.len(), 4, "two placed alone, two through the pair");
    let mut xs: Vec<f32> = screws.iter().map(|n| document.tree.world_transform(n.id).point(tequila_core::math::Vec3::ZERO).x).collect();
    xs.sort_by(f32::total_cmp);
    let first = xs[0];
    let gaps: Vec<f32> = xs.iter().map(|x| ((x - first) * 1000.0).round()).collect();
    assert_eq!(gaps, vec![0.0, 10.0, 20.0, 23.0], "placed where the blocks put them, in millimetres");
    assert!(document.tree.iter().any(|n| n.name == "Two screws"), "a named block keeps its name");
    assert!(document.tree.iter().any(|n| n.name == "Screw"), "an unnamed one takes its definition's");
}

/// A SubD cube's cage, as Rhino 7 writes it, comes in rounded.
#[test]
fn a_subd_comes_in_smooth() {
    let a = Archive { version: 70 };
    let objects = a.table(0x1000_0013, &[record(&a, 0x40000, subd_object(&a, 10.0, [0.0; 3]), attributes(&a, "22222222-0000-0000-0000-000000000001", 0, "Pebble", &[]))]);
    let bytes = a.file(2, &[objects]);
    keep("subd_cube.3dm", &bytes);
    let scene = tequila_import::rhino::read(&bytes, "model").unwrap();
    assert_eq!(scene.meshes.len(), 1, "{:?}", scene.warnings);
    let m = &scene.meshes[0];
    assert_eq!(m.indices.len() / 3, 6 * 64 * 2, "three rounds at Medium");
    assert!(m.positions.iter().all(|p| p.x > 0.0 && p.x < 10.0), "rounded inside its cage");
    assert!(volume(&scene, 0) > 0.0, "facing out");
}

/// One file with something of every kind on layers, wearing materials by
/// layer and by object: what a product model from Rhino looks like, and the
/// scene the import's pictures are rendered from.
#[test]
fn a_scene_of_every_kind_reads_whole() {
    let a = Archive { version: 80 };
    let nil = "00000000-0000-0000-0000-000000000000";
    let layers = a.table(0x1000_0011, &[
        layer(&a, "Default", [0, 0, 0, 0], true, -1, "11111111-0000-0000-0000-000000000001", nil),
        layer(&a, "Body", [40, 90, 160, 0], true, -1, "11111111-0000-0000-0000-000000000002", nil),
        layer(&a, "Trim", [240, 140, 20, 0], true, -1, "11111111-0000-0000-0000-000000000003", "11111111-0000-0000-0000-000000000002"),
        layer(&a, "Glass", [200, 230, 240, 0], true, 2, "11111111-0000-0000-0000-000000000004", nil),
        layer(&a, "Construction", [128, 128, 128, 0], false, -1, "11111111-0000-0000-0000-000000000005", nil),
    ]);
    let materials = a.table(0x1000_0010, &[
        material(&a, "Brushed steel", [180, 180, 180, 0], 0.0, 0.0, None, Some((1.0, 0.25))),
        material(&a, "Red plastic", [200, 20, 20, 0], 200.0, 0.0, None, None),
        material(&a, "Glass", [250, 250, 250, 0], 255.0, 0.95, None, None),
    ]);
    let id = |n: u32| format!("22222222-0000-0000-0000-{n:012}");
    let by_object = |material: i32| vec![(4u8, i32s(&[material])), (16u8, vec![1])];
    let bolt = "33333333-0000-0000-0000-000000000001";
    let mut component = i32s(&[1, 0]);
    component.extend_from_slice(&[0, 1]);
    component.extend_from_slice(&uuid(bolt));
    component.extend_from_slice(&[0, 0, 1]);
    component.extend(string("Bolt"));
    let mut d = a.chunk(0x4000_8002, &component);
    d.extend_from_slice(&1u32.to_le_bytes());
    let mut unit = 2u32.to_le_bytes().to_vec();
    unit.extend(f64s(&[0.001]));
    unit.extend(string(""));
    d.extend(a.anonymous(1, 0, &unit));
    for _ in 0..3 {
        d.extend(string(""));
    }
    d.extend_from_slice(&[0; 48]);
    d.push(1);
    d.extend(i32s(&[1]));
    d.extend_from_slice(&uuid(&id(100)));
    d.push(0);
    let idefs = a.table(0x1000_0021, &[a.chunk(0x2000_8076, &a.object(IDEF, &a.anonymous(1, 0, &d), &[]))]);
    let mut member = vec![0x2D];
    member.extend_from_slice(&uuid(&id(100)));
    member.extend(i32s(&[2]));
    member.extend_from_slice(&[12, 3, 0]);
    let member = a.chunk(0x0200_8072, &member);
    let place = |x: f64| {
        let mut d = vec![0x10];
        d.extend_from_slice(&uuid(bolt));
        d.extend(f64s(&[1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, -28.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]));
        d.extend_from_slice(&[0; 48]);
        a.object(IREF, &d, &[])
    };
    let objects = a.table(0x1000_0013, &[
        record(&a, 0x40000, subd_object(&a, 30.0, [0.0, 0.0, 0.0]), attributes(&a, &id(1), 1, "Pebble", &by_object(1))),
        record(&a, 0x10, ball_object(&a, 12.0, [55.0, 15.0, 12.0]), attributes(&a, &id(2), 1, "Ball", &by_object(0))),
        record(&a, 0x4000_0000, rod_object(&a, 6.0, 40.0, [85.0, 15.0, 0.0]), attributes(&a, &id(3), 2, "Rod", &[])),
        record(&a, 0x10, plate_object(&a, [-25.0, 5.0, 0.0]), attributes(&a, &id(4), 3, "Pane", &[])),
        record(&a, 0x20, cube_mesh(&a, true, [100.0, 5.0, 0.0], 14.0), attributes(&a, &id(5), 1, "Block of mesh", &[])),
        record(&a, 0x20, cube_mesh(&a, true, [0.0, 60.0, 0.0], 10.0), attributes(&a, &id(6), 4, "Jig", &[])),
        record(&a, 0x4, line_curve(&a, [0.0; 3], [10.0, 0.0, 0.0], 3), attributes(&a, &id(7), 0, "Guide", &[])),
        record(&a, 0x4000_0000, rod_object(&a, 2.0, 12.0, [0.0, 0.0, 0.0]), member),
        record(&a, 0x1000, place(10.0), attributes(&a, &id(8), 2, "", &[])),
        record(&a, 0x1000, place(35.0), attributes(&a, &id(9), 2, "", &[])),
        record(&a, 0x1000, place(60.0), attributes(&a, &id(10), 2, "", &[])),
        record(&a, 0x1000, place(85.0), attributes(&a, &id(11), 2, "", &[])),
    ]);
    let bytes = a.file(2, &[materials, layers, idefs, objects]);
    keep("showcase.3dm", &bytes);
    let scene = tequila_import::rhino::read(&bytes, "showcase").unwrap();
    assert!(scene.warnings.iter().any(|w| w.contains("curves ×1")), "{:?}", scene.warnings);
    assert_eq!(scene.warnings.len(), 1, "{:?}", scene.warnings);
    let node = |name: &str| scene.nodes.iter().find(|n| n.name == name).unwrap_or_else(|| panic!("{name}"));
    let wears = |name: &str| scene.materials[scene.meshes[node(name).mesh.unwrap()].material.unwrap()].name.clone();
    assert_eq!(wears("Pebble"), "Red plastic");
    assert_eq!(wears("Ball"), "Brushed steel");
    assert_eq!(wears("Rod"), "Body::Trim");
    assert_eq!(wears("Pane"), "Glass");
    assert_eq!(wears("Block of mesh"), "Body");
    assert!(node("Jig").hidden && !node("Ball").hidden);
    assert_eq!(scene.nodes.iter().filter(|n| n.name == "Bolt").count(), 4);
    let (document, _) = import(&bytes);
    assert_eq!(parts(&document).len(), 6 + 4);
}

/// What is not rendered is named, not silently dropped; a truncated or
/// foreign file is refused by name.
#[test]
fn what_is_left_out_is_said_and_broken_files_are_refused() {
    let a = Archive { version: 70 };
    let curve = line_curve(&a, [0.0; 3], [1.0, 0.0, 0.0], 3);
    let objects = a.table(0x1000_0013, &[
        record(&a, 0x4, curve, attributes(&a, "22222222-0000-0000-0000-000000000001", 0, "", &[])),
        record(&a, 0x20, cube_mesh(&a, true, [0.0; 3], 1.0), attributes(&a, "22222222-0000-0000-0000-000000000002", 0, "", &[])),
    ]);
    let bytes = a.file(2, &[objects]);
    let scene = tequila_import::rhino::read(&bytes, "model").unwrap();
    assert!(scene.warnings.iter().any(|w| w.contains("curves ×1")), "{:?}", scene.warnings);

    assert!(tequila_import::rhino::read(b"3D Geometry File Format        1", "x").is_err());
    assert!(tequila_import::rhino::read(b"ISO-10303-21;", "x").is_err());
    for cut in [10, 40, 120, bytes.len() / 2] {
        // Never a panic; an error or what came before the cut.
        let _ = tequila_import::rhino::read(&bytes[..cut], "x");
    }
}

/// A 3dm file read from disk is CAD: kept for dividing again when
/// something in it was divided, and divided again at another quality.
#[test]
fn a_divided_3dm_is_kept_and_divides_again() {
    let a = Archive { version: 70 };
    let objects = a.table(0x1000_0013, &[record(&a, 0x10, box_brep(&a, 10.0), attributes(&a, "22222222-0000-0000-0000-000000000001", 0, "Box", &[]))]);
    let folder = std::env::temp_dir().join(format!("tequila-3dm-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("Box.3DM");
    std::fs::write(&path, a.file(2, &[objects])).unwrap();
    let model = tequila_import::read_file(&path).expect("reads by its extension");
    assert!(model.brep.is_some(), "kept with the project");
    let (_, bytes) = model.brep.clone().unwrap();
    let again = tequila_import::read_cad(&bytes, "Box", None, Tessellation::Draft).expect("divides from its kept bytes");
    assert_eq!(again.scene.tessellation.as_deref(), Some("Draft"));
    let _ = std::fs::remove_dir_all(&folder);
}

/// Two files from McNeel's rhino3dm (`tests/data/rhino3dm`, MIT): a mesh
/// Rhino 8 saved, and a SubD with creases openNURBS 8 wrote. The numbers
/// are this reader's; what they hold it to is that a file Rhino wrote
/// reads whole, in its unit, with nothing left out.
#[test]
fn mcneel_files_read() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/rhino3dm");
    let read = |name: &str| tequila_import::rhino::read(&std::fs::read(folder.join(name)).unwrap(), name).unwrap();
    let extent = |scene: &RawScene| {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in scene.meshes.iter().flat_map(|m| m.positions.iter()) {
            for (k, c) in [p.x, p.y, p.z].into_iter().enumerate() {
                lo[k] = lo[k].min(c);
                hi[k] = hi[k].max(c);
            }
        }
        (lo, hi)
    };

    let mesh = read("mesh.3dm");
    assert!(mesh.warnings.is_empty(), "{:?}", mesh.warnings);
    assert_eq!(mesh.source_unit, Some(tequila_core::units::LengthUnit::Millimetre));
    assert_eq!(mesh.meshes.len(), 1);
    assert_eq!(mesh.meshes[0].indices.len() / 3, 276);
    let (lo, hi) = extent(&mesh);
    for k in 0..3 {
        assert!((lo[k] + 25.126).abs() < 1e-3 && (hi[k] - 25.126).abs() < 1e-3, "{lo:?} to {hi:?}");
    }

    let subd = read("subd_creases.3dm");
    assert!(subd.warnings.is_empty(), "{:?}", subd.warnings);
    assert_eq!(subd.meshes.len(), 1);
    assert_eq!(subd.meshes[0].indices.len() / 3, 768, "six cage faces, three rounds");
    let (lo, hi) = extent(&subd);
    // Inside its 10 mm cage, and down to it where the creases hold it.
    assert!(lo.iter().all(|c| c.abs() < 1e-4) && hi.iter().all(|c| *c < 10.0 && *c > 9.0), "{lo:?} to {hi:?}");
}

/// Real files, by hand: every `.3dm` in `TEQUILA_3DM_SAMPLES` reads into a
/// valid document, and says what it left out. A file of only curves,
/// points, annotation or lights renders nothing and passes; one whose
/// records, faces or pieces the reader could not read fails. Each file's
/// result is printed (`--nocapture`), and the failures listed at the end.
#[test]
#[ignore = "needs TEQUILA_3DM_SAMPLES: a folder of real .3dm files"]
fn real_files() {
    let folder = std::env::var("TEQUILA_3DM_SAMPLES").expect("TEQUILA_3DM_SAMPLES names a folder of .3dm files");
    let mut paths: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("3dm"))).collect();
    paths.sort();
    assert!(!paths.is_empty(), "no .3dm files in {folder}");
    let mut failed = Vec::new();
    for path in &paths {
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = std::fs::read(path).unwrap();
        let started = std::time::Instant::now();
        let scene = match tequila_import::rhino::read(&bytes, "sample") {
            Ok(scene) => scene,
            Err(e) => {
                println!("FAIL {file}: {e}");
                failed.push(file);
                continue;
            }
        };
        let seconds = started.elapsed().as_secs_f32();
        let triangles: usize = scene.meshes.iter().map(|m| m.indices.len() / 3).sum();
        let edges: usize = scene.meshes.iter().map(|m| m.edge_lines.len() / 2).sum();
        let parts = scene.nodes.iter().filter(|n| n.mesh.is_some()).count();
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in scene.meshes.iter().flat_map(|m| m.positions.iter()) {
            for (k, c) in [p.x, p.y, p.z].into_iter().enumerate() {
                lo[k] = lo[k].min(c);
                hi[k] = hi[k].max(c);
            }
        }
        // What the reader should have read and did not, as against what
        // it leaves out on purpose.
        let shortfall = scene.warnings.iter().any(|w| {
            w.contains("could not be read") || w.contains("could not be divided") || w.contains("pieces Tequila does not read") || w.contains("objects of class") || w.contains("ends early")
        });
        let summary = format!(
            "{file}: {parts} parts, {} meshes, {triangles} triangles, {edges} edge segments, {} materials, box {lo:?} to {hi:?} (Rhino's axes, file units), unit {:?}, {seconds:.2} s, warnings {:?}",
            scene.meshes.len(),
            scene.materials.len(),
            scene.source_unit,
            scene.warnings
        );
        let mut document = SceneDocument::new();
        let built = build(&document, None, scene, &ImportOptions::default(), None).map_err(|e| e.to_string()).and_then(|(command, _)| {
            CommandBus::new().apply(&mut document, command).map_err(|e| e.to_string())?;
            document.validate().map_err(|e| e.to_string())
        });
        match built {
            Ok(()) if !shortfall => println!("PASS {summary}"),
            Ok(()) => {
                println!("FAIL {summary}");
                failed.push(file);
            }
            Err(e) => {
                println!("FAIL {summary}; the document: {e}");
                failed.push(file);
            }
        }
    }
    println!("{} of {} files pass", paths.len() - failed.len(), paths.len());
    assert!(failed.is_empty(), "{} files fail: {failed:?}", failed.len());
}
