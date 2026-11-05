//! PLY, the Stanford polygon format: ASCII, and binary in either byte order.
//!
//! What scanners, photogrammetry and MeshLab write. The header declares
//! elements and their properties, and the body lists them in that order, so
//! the reader follows the header rather than assuming a layout:
//!
//! - `vertex` elements give the positions (`x y z`), and where the writer
//!   kept them normals (`nx ny nz`), texture coordinates (`s t`, `u v`,
//!   `texture_u texture_v`) and colours (`red green blue alpha`, or the
//!   `diffuse_` spellings), in any of the eight number types.
//! - `face` elements list their corners (`vertex_indices`, or
//!   `vertex_index`) as polygons of any size, divided into triangles here
//!   (§6.1 point 1): fanned when convex, ears clipped when not. A face may
//!   also carry a texture coordinate for each corner (MeshLab's `texcoord`),
//!   which splits a vertex wherever its corners disagree.
//! - Every other element - edges, materials, a writer's own - is read past.
//!
//! Colours are display values, 0-255 or 0-1, and are decoded to linear as
//! OBJ's are; the pipeline shows them as the part's colour. A mesh painted
//! white throughout carries nothing worth showing, and drops them.
//!
//! MeshLab names the picture a textured scan wears in a `comment
//! TextureFile` line; with texture coordinates present it becomes the
//! part's base colour map, found beside the model.
//!
//! PLY has no units and no up axis, so the model comes in as written and
//! the unit is guessed from its size (§3.4), as for OBJ and STL. A file of
//! points with no faces is refused by name: there is no surface to render.

use crate::raw::{RawMesh, RawNode, RawScene, RawTexture};
use crate::ImportError;
use tequila_core::math::{vec2, vec3, vec4, Vec2, Vec3, Vec4};
use std::collections::HashMap;

/// Read a PLY file.
pub fn read(bytes: &[u8], name: &str) -> Result<RawScene, ImportError> {
    let (header, body) = parse_header(bytes)?;
    let mut reader = Reader::new(header.format, body);

    let mut positions: Vec<Vec3> = Vec::new();
    let mut normals: Vec<Vec3> = Vec::new();
    let mut uvs: Vec<Vec2> = Vec::new();
    let mut colours: Vec<Vec4> = Vec::new();
    // Each face's corners, flattened, with where each face starts.
    let mut corners: Vec<u32> = Vec::new();
    let mut face_starts: Vec<usize> = Vec::new();
    let mut corner_uvs: Vec<Option<Vec2>> = Vec::new();
    let mut warnings = Vec::new();

    let mut vertices_read = false;
    for element in &header.elements {
        match element.name.as_str() {
            "vertex" if !vertices_read => {
                vertices_read = true;
                read_vertices(&mut reader, element, &mut positions, &mut normals, &mut uvs, &mut colours)?;
            }
            "face" => read_faces(&mut reader, element, &mut corners, &mut face_starts, &mut corner_uvs)?,
            other => {
                if other == "tristrips" {
                    warnings.push("the file holds triangle strips, which are not read; only its faces are".to_string());
                }
                for _ in 0..element.count {
                    for property in &element.properties {
                        reader.skip(property)?;
                    }
                }
            }
        }
    }

    if positions.is_empty() {
        return Err(ImportError::Malformed("the file declares no vertices".into()));
    }
    if face_starts.is_empty() {
        return Err(ImportError::Unsupported(format!(
            "this PLY is a cloud of {} points with no faces, and Tequila renders surfaces; \
             mesh it first (MeshLab's Screened Poisson, for one) and import the result",
            positions.len()
        )));
    }

    let mesh = build_mesh(&positions, &normals, &uvs, &colours, &corners, &face_starts, &corner_uvs)?;
    let mut scene = RawScene {
        nodes: vec![RawNode::part(name, 0)],
        roots: vec![0],
        meshes: vec![mesh],
        warnings,
        ..RawScene::default()
    };
    // A textured scan's picture, beside the model.
    if let Some(file) = header.texture_file.filter(|_| !scene.meshes[0].uvs.is_empty()) {
        use tequila_materials::graph::Value;
        use tequila_materials::material_type::{MaterialType, Param};
        let mut definition = tequila_scene::material::MaterialDefinition::new(
            file.rsplit(['/', '\\']).next().unwrap_or(&file).to_string(),
            MaterialType::Plastic,
            tequila_materials::openpbr::Material::plastic(tequila_core::spectrum::Spectrum::ONE, 0.6),
        );
        let wanted = RawTexture { uri: file, role: tequila_texture::texture::MapRole::Color, embedded: None, elsewhere: None };
        crate::obj::attach_texture(&mut definition, &mut scene.textures, wanted, Param::BaseColor, Value::Color(tequila_core::spectrum::Spectrum::splat(0.8)));
        scene.materials.push(definition);
        scene.meshes[0].material = Some(0);
    }
    Ok(scene)
}

/// The three encodings the format allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Ascii,
    LittleEndian,
    BigEndian,
}

/// PLY's eight number types, under either of the names each goes by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Type {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Type {
    fn parse(word: &str) -> Option<Type> {
        Some(match word {
            "char" | "int8" => Type::I8,
            "uchar" | "uint8" => Type::U8,
            "short" | "int16" => Type::I16,
            "ushort" | "uint16" => Type::U16,
            "int" | "int32" => Type::I32,
            "uint" | "uint32" => Type::U32,
            "float" | "float32" => Type::F32,
            "double" | "float64" => Type::F64,
            _ => return None,
        })
    }

    fn size(self) -> usize {
        match self {
            Type::I8 | Type::U8 => 1,
            Type::I16 | Type::U16 => 2,
            Type::I32 | Type::U32 | Type::F32 => 4,
            Type::F64 => 8,
        }
    }

    /// What full scale is for a colour stored in this type: 255 for a byte,
    /// 1 for a float.
    fn full_scale(self) -> f64 {
        match self {
            Type::U8 | Type::I8 => 255.0,
            Type::U16 | Type::I16 => 65535.0,
            Type::U32 | Type::I32 => u32::MAX as f64,
            Type::F32 | Type::F64 => 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Scalar(Type),
    /// A count of type `.0`, then that many values of type `.1`.
    List(Type, Type),
}

#[derive(Clone, Debug, PartialEq)]
struct Property {
    name: String,
    kind: Kind,
}

#[derive(Clone, Debug, PartialEq)]
struct Element {
    name: String,
    count: usize,
    properties: Vec<Property>,
}

#[derive(Debug)]
struct Header {
    format: Format,
    elements: Vec<Element>,
    /// MeshLab's `comment TextureFile name`.
    texture_file: Option<String>,
}

/// The header, and the bytes after its `end_header` line.
fn parse_header(bytes: &[u8]) -> Result<(Header, &[u8]), ImportError> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    if !bytes.starts_with(b"ply") {
        return Err(ImportError::Malformed("a PLY file starts with `ply`, and this one does not".into()));
    }
    let mut at = 0;
    let mut format = None;
    let mut elements: Vec<Element> = Vec::new();
    let mut texture_file = None;
    let mut first = true;
    loop {
        let Some(end) = bytes[at..].iter().position(|&b| b == b'\n') else {
            return Err(ImportError::Malformed("the header has no `end_header`".into()));
        };
        let line = String::from_utf8_lossy(&bytes[at..at + end]);
        at += end + 1;
        let line = line.trim();
        if first {
            first = false;
            continue;
        }
        let mut words = line.split_whitespace();
        match words.next() {
            Some("format") => {
                format = Some(match words.next() {
                    Some("ascii") => Format::Ascii,
                    Some("binary_little_endian") => Format::LittleEndian,
                    Some("binary_big_endian") => Format::BigEndian,
                    other => {
                        return Err(ImportError::Malformed(format!("`{}` is not a PLY format", other.unwrap_or(""))));
                    }
                })
            }
            Some("element") => {
                let name = words.next().unwrap_or("").to_string();
                let count = words
                    .next()
                    .and_then(|w| w.parse::<usize>().ok())
                    .ok_or_else(|| ImportError::Malformed(format!("element `{name}` has no count")))?;
                elements.push(Element { name, count, properties: Vec::new() });
            }
            Some("property") => {
                let Some(element) = elements.last_mut() else {
                    return Err(ImportError::Malformed("a property comes before any element".into()));
                };
                let words: Vec<&str> = words.collect();
                let unknown = |w: &str| ImportError::Malformed(format!("`{w}` is not a PLY number type"));
                let property = match words.as_slice() {
                    ["list", count, item, name] => Property {
                        name: name.to_string(),
                        kind: Kind::List(Type::parse(count).ok_or_else(|| unknown(count))?, Type::parse(item).ok_or_else(|| unknown(item))?),
                    },
                    [kind, name] => Property { name: name.to_string(), kind: Kind::Scalar(Type::parse(kind).ok_or_else(|| unknown(kind))?) },
                    _ => return Err(ImportError::Malformed(format!("`property {}` cannot be read", words.join(" ")))),
                };
                element.properties.push(property);
            }
            Some("comment") => {
                let rest: Vec<&str> = words.collect();
                if rest.first().is_some_and(|w| w.eq_ignore_ascii_case("TextureFile")) && rest.len() > 1 {
                    texture_file = Some(rest[1..].join(" "));
                }
            }
            Some("end_header") => break,
            // `obj_info` and blank lines.
            _ => {}
        }
    }
    let format = format.ok_or_else(|| ImportError::Malformed("the header has no `format` line".into()))?;
    Ok((Header { format, elements, texture_file }, &bytes[at..]))
}

/// Values out of the body, one at a time, in whichever encoding it is.
struct Reader<'a> {
    format: Format,
    bytes: &'a [u8],
    at: usize,
    /// ASCII: the body's words.
    words: std::iter::Peekable<std::str::SplitAsciiWhitespace<'a>>,
}

impl<'a> Reader<'a> {
    fn new(format: Format, bytes: &'a [u8]) -> Reader<'a> {
        // Lossless for any ASCII body; a binary one never takes this path.
        let text = if format == Format::Ascii { std::str::from_utf8(bytes).unwrap_or("") } else { "" };
        Reader { format, bytes, at: 0, words: text.split_ascii_whitespace().peekable() }
    }

    fn value(&mut self, kind: Type) -> Result<f64, ImportError> {
        if self.format == Format::Ascii {
            let word = self.words.next().ok_or_else(|| ImportError::Malformed("the file ends before its elements do".into()))?;
            return word
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| ImportError::Malformed(format!("`{word}` is not a finite number")));
        }
        let size = kind.size();
        let Some(slice) = self.bytes.get(self.at..self.at + size) else {
            return Err(ImportError::Malformed("the file ends before its elements do".into()));
        };
        self.at += size;
        let mut raw = [0u8; 8];
        raw[..size].copy_from_slice(slice);
        if self.format == Format::BigEndian {
            raw[..size].reverse();
        }
        let value = match kind {
            Type::I8 => raw[0] as i8 as f64,
            Type::U8 => raw[0] as f64,
            Type::I16 => i16::from_le_bytes([raw[0], raw[1]]) as f64,
            Type::U16 => u16::from_le_bytes([raw[0], raw[1]]) as f64,
            Type::I32 => i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as f64,
            Type::U32 => u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as f64,
            Type::F32 => f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as f64,
            Type::F64 => f64::from_le_bytes(raw),
        };
        if !value.is_finite() {
            return Err(ImportError::Malformed("a value in the file is not a finite number".into()));
        }
        Ok(value)
    }

    /// A list's values, its count checked against what is left so a corrupt
    /// count cannot ask for gigabytes.
    fn list(&mut self, count: Type, item: Type) -> Result<Vec<f64>, ImportError> {
        let n = self.value(count)?;
        if n < 0.0 || n.fract() != 0.0 {
            return Err(ImportError::Malformed(format!("a list claims {n} entries")));
        }
        let n = n as usize;
        let left = if self.format == Format::Ascii { usize::MAX } else { (self.bytes.len() - self.at) / item.size() };
        if n > left {
            return Err(ImportError::Malformed("a list runs past the end of the file".into()));
        }
        (0..n).map(|_| self.value(item)).collect()
    }

    fn skip(&mut self, property: &Property) -> Result<(), ImportError> {
        match property.kind {
            Kind::Scalar(t) => self.value(t).map(|_| ()),
            Kind::List(count, item) => self.list(count, item).map(|_| ()),
        }
    }

    /// Whether `count` elements could still fit in what is left: at least
    /// `bytes_each` bytes apiece in binary, and in ASCII a digit and a space
    /// for each of their `values_each` values. A header claiming billions of
    /// vertices in a small file is refused before anything is reserved.
    fn could_hold(&self, count: usize, bytes_each: usize, values_each: usize) -> bool {
        let each = if self.format == Format::Ascii { 2 * values_each.max(1) } else { bytes_each.max(1) };
        count.saturating_mul(each) <= self.bytes.len() - self.at + 1
    }
}

/// Where in a vertex's properties each channel is, by the names writers use.
struct Layout {
    position: [Option<usize>; 3],
    normal: [Option<usize>; 3],
    uv: [Option<usize>; 2],
    colour: [Option<usize>; 4],
}

impl Layout {
    fn of(element: &Element) -> Layout {
        let find = |names: &[&str]| element.properties.iter().position(|p| matches!(p.kind, Kind::Scalar(_)) && names.contains(&p.name.as_str()));
        Layout {
            position: [find(&["x"]), find(&["y"]), find(&["z"])],
            normal: [find(&["nx", "normal_x"]), find(&["ny", "normal_y"]), find(&["nz", "normal_z"])],
            uv: [
                find(&["s", "u", "texture_u", "texture_s", "tx"]),
                find(&["t", "v", "texture_v", "texture_t", "ty"]),
            ],
            colour: [
                find(&["red", "r", "diffuse_red"]),
                find(&["green", "g", "diffuse_green"]),
                find(&["blue", "b", "diffuse_blue"]),
                find(&["alpha", "a", "diffuse_alpha"]),
            ],
        }
    }
}

fn read_vertices(
    reader: &mut Reader,
    element: &Element,
    positions: &mut Vec<Vec3>,
    normals: &mut Vec<Vec3>,
    uvs: &mut Vec<Vec2>,
    colours: &mut Vec<Vec4>,
) -> Result<(), ImportError> {
    let layout = Layout::of(element);
    let [Some(x), Some(y), Some(z)] = layout.position else {
        return Err(ImportError::Malformed("the vertices have no x, y and z".into()));
    };
    let has_normals = layout.normal.iter().all(Option::is_some);
    let has_uvs = layout.uv.iter().all(Option::is_some);
    let has_colours = layout.colour[..3].iter().all(Option::is_some);
    let scale_of = |i: Option<usize>| match i.map(|i| &element.properties[i].kind) {
        Some(Kind::Scalar(t)) => t.full_scale(),
        _ => 1.0,
    };
    let colour_scale = [scale_of(layout.colour[0]), scale_of(layout.colour[1]), scale_of(layout.colour[2]), scale_of(layout.colour[3])];
    let smallest: usize = element.properties.iter().map(|p| match p.kind { Kind::Scalar(t) => t.size(), Kind::List(c, _) => c.size() }).sum();
    if !reader.could_hold(element.count, smallest, element.properties.len()) {
        return Err(ImportError::Malformed(format!("the header declares {} vertices, more than the file holds", element.count)));
    }
    positions.reserve(element.count);
    let mut values = vec![0.0f64; element.properties.len()];
    for _ in 0..element.count {
        for (i, property) in element.properties.iter().enumerate() {
            values[i] = match property.kind {
                Kind::Scalar(t) => reader.value(t)?,
                Kind::List(count, item) => {
                    reader.list(count, item)?;
                    0.0
                }
            };
        }
        positions.push(vec3(values[x] as f32, values[y] as f32, values[z] as f32));
        if has_normals {
            let [nx, ny, nz] = layout.normal.map(|i| values[i.unwrap()] as f32);
            normals.push(vec3(nx, ny, nz));
        }
        if has_uvs {
            // Texture coordinates have their origin at the bottom left, as
            // OBJ's do; the renderer's is at the top left.
            uvs.push(vec2(values[layout.uv[0].unwrap()] as f32, 1.0 - values[layout.uv[1].unwrap()] as f32));
        }
        if has_colours {
            let channel = |c: usize| (values[layout.colour[c].unwrap()] / colour_scale[c]).clamp(0.0, 1.0) as f32;
            let alpha = layout.colour[3].map_or(1.0, |i| (values[i] / colour_scale[3]).clamp(0.0, 1.0) as f32);
            colours.push(vec4(srgb_to_linear(channel(0)), srgb_to_linear(channel(1)), srgb_to_linear(channel(2)), alpha));
        }
    }
    // White throughout says nothing, and would put a vertex colour node on
    // the material for no change.
    if colours.iter().all(|c| c.x > 0.999 && c.y > 0.999 && c.z > 0.999) {
        colours.clear();
    }
    Ok(())
}

fn read_faces(
    reader: &mut Reader,
    element: &Element,
    corners: &mut Vec<u32>,
    face_starts: &mut Vec<usize>,
    corner_uvs: &mut Vec<Option<Vec2>>,
) -> Result<(), ImportError> {
    let indices = element
        .properties
        .iter()
        .position(|p| matches!(p.kind, Kind::List(..)) && (p.name == "vertex_indices" || p.name == "vertex_index"))
        .ok_or_else(|| ImportError::Malformed("the faces have no `vertex_indices` list".into()))?;
    let texcoords = element.properties.iter().position(|p| matches!(p.kind, Kind::List(..)) && p.name == "texcoord");
    // A face is at least its count.
    if !reader.could_hold(element.count, 1, 1) {
        return Err(ImportError::Malformed(format!("the header declares {} faces, more than the file holds", element.count)));
    }
    for face in 0..element.count {
        let mut list = Vec::new();
        let mut uv_list = Vec::new();
        for (i, property) in element.properties.iter().enumerate() {
            match property.kind {
                Kind::List(count, item) if i == indices => list = reader.list(count, item)?,
                Kind::List(count, item) if Some(i) == texcoords => uv_list = reader.list(count, item)?,
                _ => reader.skip(property)?,
            }
        }
        if list.len() < 3 {
            // A point or an edge, which some writers keep among the faces.
            continue;
        }
        face_starts.push(corners.len());
        let wedge = uv_list.len() == list.len() * 2;
        for (k, &index) in list.iter().enumerate() {
            if index < 0.0 || index > u32::MAX as f64 {
                return Err(ImportError::Malformed(format!("face {face} names vertex {index}")));
            }
            corners.push(index as u32);
            corner_uvs.push(wedge.then(|| vec2(uv_list[2 * k] as f32, 1.0 - uv_list[2 * k + 1] as f32)));
        }
    }
    Ok(())
}

/// The vertices and faces as one triangle mesh: faces divided, and a vertex
/// split wherever a face gives its corner a texture coordinate of its own.
fn build_mesh(
    positions: &[Vec3],
    normals: &[Vec3],
    uvs: &[Vec2],
    colours: &[Vec4],
    corners: &[u32],
    face_starts: &[usize],
    corner_uvs: &[Option<Vec2>],
) -> Result<RawMesh, ImportError> {
    let wedges = corner_uvs.iter().any(Option::is_some);
    let mut mesh = RawMesh::default();
    // Without corner coordinates the file's own vertices are the mesh's;
    // with them a vertex is (index, coordinate) and is made once per pair.
    let mut made: HashMap<(u32, [u32; 2]), u32> = HashMap::new();
    let mut vertex_of = |mesh: &mut RawMesh, corner: usize| -> u32 {
        let index = corners[corner];
        let uv = if wedges { corner_uvs[corner].unwrap_or(vec2(0.0, 0.0)) } else { uvs.get(index as usize).copied().unwrap_or(vec2(0.0, 0.0)) };
        let key = (index, if wedges { [uv.x.to_bits(), uv.y.to_bits()] } else { [0, 0] });
        *made.entry(key).or_insert_with(|| {
            let i = index as usize;
            mesh.positions.push(positions[i]);
            if !normals.is_empty() {
                mesh.normals.push(normals[i]);
            }
            if wedges || !uvs.is_empty() {
                mesh.uvs.push(uv);
            }
            if !colours.is_empty() {
                mesh.colors.push(colours[i]);
            }
            (mesh.positions.len() - 1) as u32
        })
    };
    for (f, &start) in face_starts.iter().enumerate() {
        let end = face_starts.get(f + 1).copied().unwrap_or(corners.len());
        if let Some(bad) = corners[start..end].iter().find(|&&i| i as usize >= positions.len()) {
            return Err(ImportError::Malformed(format!(
                "face {f} names vertex {bad}, and there are only {}",
                positions.len()
            )));
        }
        let points: Vec<Vec3> = corners[start..end].iter().map(|&i| positions[i as usize]).collect();
        for [a, b, c] in triangulate_polygon(&points) {
            for k in [a, b, c] {
                let v = vertex_of(&mut mesh, start + k);
                mesh.indices.push(v);
            }
        }
    }
    Ok(mesh)
}

/// A polygon's corners as triangles, by index into `points`, wound as the
/// polygon is.
///
/// Fanned from the first corner when the polygon is convex, which nearly
/// every face in a scan or a CAD export is and which keeps a quad's two
/// triangles where its writer put them. A concave one has its ears clipped
/// in its own plane, since a fan across a notch makes triangles outside the
/// face; one that will not clip (twisted, or with repeated corners) falls
/// back to the fan rather than losing the face.
pub(crate) fn triangulate_polygon(points: &[Vec3]) -> Vec<[usize; 3]> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    let fan = || (1..n - 1).map(|i| [0, i, i + 1]).collect::<Vec<_>>();
    if n == 3 {
        return fan();
    }
    // Newell's normal: the polygon's facing, robust to a slight warp.
    let mut normal = Vec3::ZERO;
    for i in 0..n {
        let (p, q) = (points[i], points[(i + 1) % n]);
        normal = normal + vec3((p.y - q.y) * (p.z + q.z), (p.z - q.z) * (p.x + q.x), (p.x - q.x) * (p.y + q.y));
    }
    if normal.length() < 1e-20 {
        return fan();
    }
    let turn = |a: Vec3, b: Vec3, c: Vec3| (b - a).cross(c - b).dot(normal);
    if (0..n).all(|i| turn(points[i], points[(i + 1) % n], points[(i + 2) % n]) >= 0.0) {
        return fan();
    }

    let mut left: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n - 2);
    let mut guard = 0;
    while left.len() > 3 && guard < n * n {
        guard += 1;
        let m = left.len();
        let ear = (0..m).find(|&i| {
            let (a, b, c) = (left[(i + m - 1) % m], left[i], left[(i + 1) % m]);
            if turn(points[a], points[b], points[c]) <= 0.0 {
                return false;
            }
            // No other corner inside the triangle.
            !left.iter().any(|&p| {
                p != a && p != b && p != c && {
                    let q = points[p];
                    turn(points[a], points[b], q) >= 0.0 && turn(points[b], points[c], q) >= 0.0 && turn(points[c], points[a], q) >= 0.0
                }
            })
        });
        let Some(i) = ear else {
            return fan();
        };
        out.push([left[(i + m - 1) % m], left[i], left[(i + 1) % m]]);
        left.remove(i);
    }
    if left.len() == 3 {
        out.push([left[0], left[1], left[2]]);
    }
    out
}

/// A display value, 0 to 1, as linear light.
pub(crate) fn srgb_to_linear(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUBE_ASCII: &str = "ply
format ascii 1.0
comment a unit cube, quads
element vertex 8
property float x
property float y
property float z
element face 6
property list uchar int vertex_indices
end_header
0 0 0
1 0 0
1 1 0
0 1 0
0 0 1
1 0 1
1 1 1
0 1 1
4 0 3 2 1
4 4 5 6 7
4 0 1 5 4
4 2 3 7 6
4 1 2 6 5
4 3 0 4 7
";

    #[test]
    fn an_ascii_cube_of_quads_reads() {
        let scene = read(CUBE_ASCII.as_bytes(), "cube").unwrap();
        let mesh = &scene.meshes[0];
        assert_eq!(mesh.positions.len(), 8);
        assert_eq!(mesh.indices.len(), 36, "six quads are twelve triangles");
        assert!(mesh.normals.is_empty() && mesh.uvs.is_empty() && mesh.colors.is_empty());
        assert_eq!(scene.nodes[0].name, "cube");
        assert_eq!(scene.source_unit, None, "PLY has no units");
    }

    /// The same vertex and face records, binary, in both byte orders, with
    /// normals, colours as bytes and texture coordinates, and a list count
    /// of a type other than `uchar`.
    fn binary(big: bool) -> Vec<u8> {
        let mut out = format!(
            "ply\nformat {} 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\n\
             property float nx\nproperty float ny\nproperty float nz\nproperty uchar red\nproperty uchar green\nproperty uchar blue\n\
             property float s\nproperty float t\nelement face 1\nproperty list ushort uint vertex_indices\nend_header\n",
            if big { "binary_big_endian" } else { "binary_little_endian" }
        )
        .into_bytes();
        let f = |out: &mut Vec<u8>, v: f32| out.extend_from_slice(&if big { v.to_be_bytes() } else { v.to_le_bytes() });
        for (p, rgb, st) in [([0.0, 0.0, 0.0], [255u8, 0, 0], [0.0, 0.0]), ([2.0, 0.0, 0.0], [0, 255, 0], [1.0, 0.0]), ([0.0, 3.0, 0.0], [128, 128, 128], [0.0, 1.0])] {
            for v in p {
                f(&mut out, v);
            }
            for v in [0.0, 0.0, 1.0] {
                f(&mut out, v);
            }
            out.extend_from_slice(&rgb);
            for v in st {
                f(&mut out, v);
            }
        }
        out.extend_from_slice(&if big { 3u16.to_be_bytes() } else { 3u16.to_le_bytes() });
        for i in [0u32, 1, 2] {
            out.extend_from_slice(&if big { i.to_be_bytes() } else { i.to_le_bytes() });
        }
        out
    }

    #[test]
    fn binary_files_read_in_both_byte_orders() {
        for big in [false, true] {
            let scene = read(&binary(big), "tri").unwrap();
            let mesh = &scene.meshes[0];
            assert_eq!(mesh.positions, vec![vec3(0.0, 0.0, 0.0), vec3(2.0, 0.0, 0.0), vec3(0.0, 3.0, 0.0)], "big endian: {big}");
            assert_eq!(mesh.indices, vec![0, 1, 2]);
            assert_eq!(mesh.normals[1], vec3(0.0, 0.0, 1.0));
            // Flipped to a top-left origin.
            assert_eq!(mesh.uvs[2], vec2(0.0, 0.0));
            assert_eq!(mesh.uvs[0], vec2(0.0, 1.0));
            // Bytes, decoded from display values: 128 is about 0.216 linear.
            assert!((mesh.colors[0].x - 1.0).abs() < 1e-6 && mesh.colors[0].y.abs() < 1e-6);
            assert!((mesh.colors[2].x - 0.216).abs() < 0.002, "{:?}", mesh.colors[2]);
        }
    }

    #[test]
    fn a_concave_polygon_is_clipped_into_triangles_inside_it() {
        // An L of six corners: a fan from the first corner would cover the
        // notch.
        let l = [vec3(0.0, 0.0, 0.0), vec3(2.0, 0.0, 0.0), vec3(2.0, 1.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(1.0, 2.0, 0.0), vec3(0.0, 2.0, 0.0)];
        let triangles = triangulate_polygon(&l);
        assert_eq!(triangles.len(), 4);
        let area: f32 = triangles.iter().map(|&[a, b, c]| (l[b] - l[a]).cross(l[c] - l[a]).z * 0.5).sum();
        assert!((area - 3.0).abs() < 1e-5, "the triangles cover {area}, not the L's 3");
        for &[a, b, c] in &triangles {
            assert!((l[b] - l[a]).cross(l[c] - l[a]).z > 0.0, "a triangle is wound against the face");
            let centre = (l[a] + l[b] + l[c]) * (1.0 / 3.0);
            assert!(!(centre.x > 1.0 && centre.y > 1.0), "a triangle lies in the notch");
        }
        // And a convex pentagon is fanned, in order.
        let pentagon: Vec<Vec3> = (0..5).map(|i| { let a = i as f32 / 5.0 * std::f32::consts::TAU; vec3(a.cos(), a.sin(), 0.0) }).collect();
        assert_eq!(triangulate_polygon(&pentagon), vec![[0, 1, 2], [0, 2, 3], [0, 3, 4]]);
    }

    #[test]
    fn corner_texture_coordinates_split_vertices_and_name_the_picture() {
        let text = "ply
format ascii 1.0
comment TextureFile scan colour.jpg
element vertex 4
property float x
property float y
property float z
element face 2
property list uchar int vertex_indices
property list uchar float texcoord
end_header
0 0 0
1 0 0
1 1 0
0 1 0
3 0 1 2 6 0 0 1 0 1 1
3 0 2 3 6 0.5 0 0.5 1 0 1
";
        let scene = read(text.as_bytes(), "scan").unwrap();
        let mesh = &scene.meshes[0];
        // Corner 0 has two coordinates, corner 2 two: six vertices, not four.
        assert_eq!(mesh.positions.len(), 6);
        assert_eq!(mesh.uvs.len(), 6);
        assert_eq!(scene.textures.len(), 1);
        assert_eq!(scene.textures[0].uri, "scan colour.jpg");
        assert_eq!(scene.meshes[0].material, Some(0));
    }

    #[test]
    fn other_elements_are_read_past_and_points_alone_are_refused() {
        let text = "ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
element edge 1
property int vertex1
property int vertex2
element face 1
property list uchar int vertex_indices
property uchar flags
end_header
0 0 0
1 0 0
0 1 0
0 1
3 0 1 2 7
";
        assert_eq!(read(text.as_bytes(), "t").unwrap().meshes[0].indices, vec![0, 1, 2]);

        let points = "ply\nformat ascii 1.0\nelement vertex 2\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n1 1 1\n";
        let error = read(points.as_bytes(), "cloud").unwrap_err();
        assert!(matches!(error, ImportError::Unsupported(_)), "{error}");
        assert!(error.to_string().contains("points"), "{error}");
    }

    #[test]
    fn white_vertex_colours_are_dropped() {
        let text = "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\n\
                    property uchar red\nproperty uchar green\nproperty uchar blue\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n\
                    0 0 0 255 255 255\n1 0 0 255 255 255\n0 1 0 255 255 255\n3 0 1 2\n";
        assert!(read(text.as_bytes(), "t").unwrap().meshes[0].colors.is_empty());
    }

    #[test]
    fn broken_files_are_errors_not_panics() {
        let mut truncated = binary(false);
        truncated.truncate(truncated.len() - 5);
        assert!(read(&truncated, "t").is_err());
        for bad in [
            "",
            "plyx",
            "ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\n",
            "ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n3 0 1 2\n",
            "ply\nformat ascii 1.0\nelement vertex 1\nproperty quaternion x\nend_header\n",
            "ply\nformat binary_little_endian 1.0\nelement vertex 4000000000\nproperty float x\nproperty float y\nproperty float z\nend_header\n",
            "ply\nformat ascii 1.0\nelement vertex 4000000000\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
        ] {
            assert!(read(bad.as_bytes(), "t").is_err(), "{bad:?}");
        }
        // A list claiming a quarter of a billion entries in a tiny file.
        let mut huge = b"ply\nformat binary_little_endian 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uint int vertex_indices\nend_header\n".to_vec();
        huge.extend_from_slice(&[0u8; 36]);
        huge.extend_from_slice(&0x1000_0000u32.to_le_bytes());
        assert!(read(&huge, "t").is_err());
    }
}
