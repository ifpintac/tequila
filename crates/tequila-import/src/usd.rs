//! USD import (`docs/03` §6.3): the text form (`.usda`), the binary form
//! (`.usdc`, through `tequila_format::usdc`) and the USDZ package of either
//! with its pictures (`.usdz`).
//!
//! The text form as Tequila writes it and as Blender, Houdini and usdcat
//! write it: prims and their typed attributes, relationships, and metadata,
//! with nothing it does not need parsed beyond skipping it. A binary layer
//! is read into the same prims, so everything below holds for both.
//!
//! * An `Xform` (and any other prim with children) is a group, a `Mesh` a
//!   part, each where its `xformOp`s put it - a matrix, or translate,
//!   rotate (`rotateXYZ` and its orders, `orient`) and scale in
//!   `xformOpOrder`.
//! * A mesh's polygons are fanned into triangles; normals and `primvars:st`
//!   (or any `texCoord2f[] primvars:*`) are read `vertex` or `faceVarying`,
//!   indexed or not.
//! * A material is, in order of preference: the Tequila definition a
//!   Tequila file carries (`tequila:definition`), every parameter of an
//!   OpenPBR MaterialX shader, or a `UsdPreviewSurface`'s - with the images
//!   its inputs are connected to (`UsdUVTexture`, a channel of one or all
//!   three) as image nodes in its graph: from inside the package for a
//!   USDZ, beside the file for a loose layer.
//! * `upAxis = "Z"` turns the stage Y up; `metersPerUnit` is the source's
//!   unit.
//! * A reference to a prim in the same file is followed - what USD's
//!   instancing is written with, a prototype under a `class` prim that each
//!   copy references - and `class` prims themselves are not drawn.
//!
//! References to other files, payloads and variants are not followed: what
//! is in the file is what is read.

use crate::raw::{RawMesh, RawNode, RawScene, RawTexture};
use crate::ImportError;
use tequila_core::math::{vec2, vec3, Mat4, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_core::units::LengthUnit;
use tequila_materials::material_type::{MaterialType, Param, ParamValue};
use tequila_scene::material::MaterialDefinition;
use std::collections::BTreeMap;

// ---- tokens ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Word(String),
    Number(f64),
    Text(String),
    Path(String),
    Asset(String),
    Punct(char),
}

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '"' || c == '\'' {
            // A string, or a triple-quoted one.
            let triple = i + 2 < chars.len() && chars[i + 1] == c && chars[i + 2] == c;
            i += if triple { 3 } else { 1 };
            let mut s = String::new();
            loop {
                if i >= chars.len() {
                    return Err("a string runs to the end of the file".into());
                }
                if triple && chars[i] == c && i + 2 < chars.len() && chars[i + 1] == c && chars[i + 2] == c {
                    i += 3;
                    break;
                }
                if !triple && chars[i] == c {
                    i += 1;
                    break;
                }
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                    s.push(match chars[i] {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                } else {
                    s.push(chars[i]);
                }
                i += 1;
            }
            out.push(Token::Text(s));
        } else if c == '<' {
            let end = chars[i..].iter().position(|&c| c == '>').ok_or("a path is not closed")?;
            out.push(Token::Path(chars[i + 1..i + end].iter().collect()));
            i += end + 1;
        } else if c == '@' {
            let end = chars[i + 1..].iter().position(|&c| c == '@').ok_or("an asset path is not closed")?;
            out.push(Token::Asset(chars[i + 1..i + 1 + end].iter().collect()));
            i += end + 2;
        } else if c.is_ascii_digit() || ((c == '-' || c == '+' || c == '.') && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit() || *d == '.')) {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '.' | '-' | '+')) {
                // An exponent's sign belongs to the number; any other sign
                // ends it.
                if matches!(chars[i], '-' | '+') && !matches!(chars[i - 1], 'e' | 'E') {
                    break;
                }
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            match text.parse::<f64>() {
                Ok(v) => out.push(Token::Number(v)),
                Err(_) => out.push(Token::Word(text)),
            }
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | ':' | '.')) {
                i += 1;
            }
            out.push(Token::Word(chars[start..i].iter().collect()));
        } else {
            out.push(Token::Punct(c));
            i += 1;
        }
    }
    Ok(out)
}

// ---- values and prims ---------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Number(f64),
    Text(String),
    Path(String),
    List(Vec<Val>),
    Word(String),
    None,
}

impl Val {
    fn numbers(&self) -> Vec<f64> {
        match self {
            Val::Number(n) => vec![*n],
            Val::List(items) => items.iter().flat_map(|v| v.numbers()).collect(),
            _ => Vec::new(),
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Val::Text(s) | Val::Word(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Attribute {
    value: Option<Val>,
    /// `interpolation` and the like, from the parentheses after it.
    meta: BTreeMap<String, Val>,
}

#[derive(Clone, Debug, Default)]
struct Prim {
    /// `def`, `over` or `class`.
    specifier: String,
    kind: String,
    name: String,
    /// The prim's own metadata: `references`, `kind`, `instanceable`.
    meta: BTreeMap<String, Val>,
    attributes: BTreeMap<String, Attribute>,
    children: Vec<Prim>,
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    /// How deep in prims and values the parser is. A file nested tens of
    /// thousands deep ran the reading thread out of stack, which ends the
    /// application; past [`DEEPEST`] the rest of the file is let go.
    depth: usize,
}

/// Deeper than any scene is written, and well inside the stack.
const DEEPEST: usize = 256;

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.at).cloned();
        self.at += 1;
        t
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(&Token::Punct(c)) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    /// Past a bracketed run, `open` already taken.
    fn skip_group(&mut self, open: char, close: char) {
        let mut depth = 1;
        while let Some(t) = self.next() {
            match t {
                Token::Punct(c) if c == open => depth += 1,
                Token::Punct(c) if c == close => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    fn value(&mut self) -> Val {
        if self.depth >= DEEPEST {
            self.at = self.tokens.len();
            return Val::None;
        }
        self.depth += 1;
        let value = self.value_within();
        self.depth -= 1;
        value
    }

    fn value_within(&mut self) -> Val {
        match self.next() {
            Some(Token::Number(n)) => Val::Number(n),
            Some(Token::Text(s)) => Val::Text(s),
            Some(Token::Asset(s)) => Val::Text(s),
            Some(Token::Path(p)) => Val::Path(p),
            Some(Token::Word(w)) if w == "None" => Val::None,
            Some(Token::Word(w)) => Val::Word(w),
            Some(Token::Punct('[')) => {
                let mut items = Vec::new();
                while !self.eat(']') {
                    if self.peek().is_none() {
                        break;
                    }
                    items.push(self.value());
                    self.eat(',');
                }
                Val::List(items)
            }
            Some(Token::Punct('(')) => {
                let mut items = Vec::new();
                while !self.eat(')') {
                    if self.peek().is_none() {
                        break;
                    }
                    items.push(self.value());
                    self.eat(',');
                }
                Val::List(items)
            }
            Some(Token::Punct('{')) => {
                self.skip_group('{', '}');
                Val::None
            }
            Some(Token::Punct('-')) => match self.value() {
                Val::Number(n) => Val::Number(-n),
                other => other,
            },
            _ => Val::None,
        }
    }

    /// A parenthesised `key = value` list: a prim's or an attribute's
    /// metadata.
    fn metadata(&mut self) -> BTreeMap<String, Val> {
        let mut out = BTreeMap::new();
        while !self.eat(')') {
            match self.next() {
                None => break,
                Some(Token::Word(key)) => {
                    // `prepend apiSchemas = [...]` and its kin.
                    let key = if matches!(key.as_str(), "prepend" | "append" | "add" | "delete" | "reorder") {
                        match self.next() {
                            Some(Token::Word(k)) => k,
                            _ => key,
                        }
                    } else {
                        key
                    };
                    if self.eat('=') {
                        let v = self.value();
                        out.insert(key, v);
                    }
                }
                Some(Token::Text(_)) => {}
                Some(Token::Punct('(')) => self.skip_group('(', ')'),
                Some(Token::Punct('{')) => self.skip_group('{', '}'),
                Some(Token::Punct('[')) => self.skip_group('[', ']'),
                _ => {}
            }
        }
        out
    }

    /// The prims and properties until the closing brace.
    fn body(&mut self, prim: &mut Prim) {
        loop {
            match self.peek().cloned() {
                None => return,
                Some(Token::Punct('}')) => {
                    self.at += 1;
                    return;
                }
                Some(Token::Word(w)) if matches!(w.as_str(), "def" | "over" | "class") => {
                    self.at += 1;
                    if let Some(mut child) = self.prim() {
                        child.specifier = w;
                        prim.children.push(child);
                    }
                }
                Some(Token::Word(w)) if w == "variantSet" => {
                    // Not followed: skip it whole.
                    while let Some(t) = self.next() {
                        if t == Token::Punct('{') {
                            self.skip_group('{', '}');
                            break;
                        }
                    }
                }
                Some(Token::Word(_)) => self.property(prim),
                Some(_) => {
                    self.at += 1;
                }
            }
        }
    }

    /// One prim, after its specifier.
    fn prim(&mut self) -> Option<Prim> {
        if self.depth >= DEEPEST {
            self.at = self.tokens.len();
            return None;
        }
        self.depth += 1;
        let prim = self.prim_within();
        self.depth -= 1;
        prim
    }

    fn prim_within(&mut self) -> Option<Prim> {
        let mut prim = Prim::default();
        match self.next()? {
            Token::Word(kind) => {
                prim.kind = kind;
                if let Some(Token::Text(name)) = self.next() {
                    prim.name = name;
                }
            }
            Token::Text(name) => prim.name = name,
            _ => return None,
        }
        if self.eat('(') {
            prim.meta = self.metadata();
        }
        if self.eat('{') {
            self.body(&mut prim);
        }
        Some(prim)
    }

    /// One attribute or relationship: qualifiers, a type, a name, perhaps
    /// a value and metadata.
    fn property(&mut self, prim: &mut Prim) {
        let mut words = Vec::new();
        while let Some(Token::Word(w)) = self.peek().cloned() {
            self.at += 1;
            words.push(w);
            if self.peek() == Some(&Token::Punct('[')) && self.tokens.get(self.at + 1) == Some(&Token::Punct(']')) {
                self.at += 2;
                if let Some(last) = words.last_mut() {
                    last.push_str("[]");
                }
            }
        }
        let Some(name) = words.last().cloned() else {
            self.at += 1;
            return;
        };
        let mut attribute = Attribute::default();
        if self.eat('=') {
            attribute.value = Some(self.value());
        }
        if self.eat('(') {
            attribute.meta = self.metadata();
        }
        if words.len() >= 2 || attribute.value.is_some() {
            prim.attributes.insert(name, attribute);
        }
    }
}

// ---- building the scene -------------------------------------------------------

fn rotation(axis: usize, degrees: f64) -> Mat4 {
    let v = match axis {
        0 => vec3(1.0, 0.0, 0.0),
        1 => vec3(0.0, 1.0, 0.0),
        _ => vec3(0.0, 0.0, 1.0),
    };
    Mat4::rotation(v, (degrees as f32).to_radians())
}

/// A row-vector USD matrix as Tequila's column-vector one.
fn usd_matrix(values: &[f64]) -> Option<Mat4> {
    (values.len() == 16).then(|| {
        let mut m = Mat4::IDENTITY;
        for r in 0..4 {
            for c in 0..4 {
                m.m[r][c] = values[c * 4 + r] as f32;
            }
        }
        m
    })
}

/// A prim's place in its parent, from its `xformOp`s in their order.
fn transform_of(prim: &Prim) -> Transform {
    let order: Vec<String> = match prim.attributes.get("xformOpOrder").and_then(|a| a.value.clone()) {
        Some(Val::List(items)) => items.iter().filter_map(|v| v.text().map(str::to_string)).collect(),
        _ => Vec::new(),
    };
    let mut m = Mat4::IDENTITY;
    for op in order {
        let (inverse, name) = match op.strip_prefix("!invert!") {
            Some(rest) => (true, rest.to_string()),
            None => (false, op),
        };
        let Some(value) = prim.attributes.get(&name).and_then(|a| a.value.clone()) else { continue };
        let n = value.numbers();
        let kind = name.trim_start_matches("xformOp:").split(':').next().unwrap_or("");
        let step = match kind {
            "transform" => usd_matrix(&n),
            "translate" if n.len() >= 3 => Some(Mat4::translation(vec3(n[0] as f32, n[1] as f32, n[2] as f32))),
            "scale" if n.len() >= 3 => Some(Mat4::scale(vec3(n[0] as f32, n[1] as f32, n[2] as f32))),
            "scale" if n.len() == 1 => Some(Mat4::scale(Vec3::splat(n[0] as f32))),
            "rotateX" if !n.is_empty() => Some(rotation(0, n[0])),
            "rotateY" if !n.is_empty() => Some(rotation(1, n[0])),
            "rotateZ" if !n.is_empty() => Some(rotation(2, n[0])),
            k if k.starts_with("rotate") && k.len() == 9 && n.len() >= 3 => {
                // rotateXYZ: X first, then Y, then Z, applied to a point.
                let axes: Vec<usize> = k[6..].chars().map(|c| match c { 'X' => 0, 'Y' => 1, _ => 2 }).collect();
                let mut r = Mat4::IDENTITY;
                for (i, &axis) in axes.iter().enumerate() {
                    r = rotation(axis, n[i]) * r;
                }
                Some(r)
            }
            "orient" if n.len() >= 4 => {
                let (w, x, y, z) = (n[0] as f32, n[1] as f32, n[2] as f32, n[3] as f32);
                let mut q = Mat4::IDENTITY;
                q.m[0][0] = 1.0 - 2.0 * (y * y + z * z);
                q.m[0][1] = 2.0 * (x * y - z * w);
                q.m[0][2] = 2.0 * (x * z + y * w);
                q.m[1][0] = 2.0 * (x * y + z * w);
                q.m[1][1] = 1.0 - 2.0 * (x * x + z * z);
                q.m[1][2] = 2.0 * (y * z - x * w);
                q.m[2][0] = 2.0 * (x * z - y * w);
                q.m[2][1] = 2.0 * (y * z + x * w);
                q.m[2][2] = 1.0 - 2.0 * (x * x + y * y);
                Some(q)
            }
            _ => None,
        };
        if let Some(step) = step {
            let step = if inverse { step.inverse().unwrap_or(step) } else { step };
            m = m * step;
        }
    }
    Transform::from_matrix(m).unwrap_or(Transform::IDENTITY)
}

/// A mesh prim's triangles, every channel per corner.
fn mesh_of(prim: &Prim) -> Option<RawMesh> {
    let get = |name: &str| prim.attributes.get(name);
    let points = get("points")?.value.as_ref()?.numbers();
    let counts: Vec<usize> = get("faceVertexCounts")?.value.as_ref()?.numbers().into_iter().map(|n| n as usize).collect();
    let corners: Vec<usize> = get("faceVertexIndices")?.value.as_ref()?.numbers().into_iter().map(|n| n as usize).collect();
    let positions: Vec<Vec3> = points.chunks_exact(3).map(|p| vec3(p[0] as f32, p[1] as f32, p[2] as f32)).collect();
    if corners.iter().any(|&i| i >= positions.len()) {
        return None;
    }
    // A channel, as one value per corner, from whichever interpolation it
    // was written with.
    let per_corner = |attribute: &Attribute, width: usize, indices: Option<Vec<usize>>| -> Option<Vec<Vec<f32>>> {
        let values: Vec<f32> = attribute.value.as_ref()?.numbers().into_iter().map(|v| v as f32).collect();
        let items: Vec<Vec<f32>> = values.chunks_exact(width).map(|c| c.to_vec()).collect();
        let items: Vec<Vec<f32>> = match indices {
            Some(indices) => indices.iter().filter_map(|&i| items.get(i).cloned()).collect(),
            None => items,
        };
        let interpolation = attribute.meta.get("interpolation").and_then(|v| v.text()).unwrap_or("vertex");
        match interpolation {
            "faceVarying" if items.len() == corners.len() => Some(items),
            "vertex" | "varying" if items.len() == positions.len() => Some(corners.iter().map(|&i| items[i].clone()).collect()),
            "uniform" if items.len() == counts.len() => Some(counts.iter().enumerate().flat_map(|(f, &n)| std::iter::repeat_n(items[f].clone(), n)).collect()),
            "constant" if !items.is_empty() => Some(vec![items[0].clone(); corners.len()]),
            _ if items.len() == corners.len() => Some(items),
            _ if items.len() == positions.len() => Some(corners.iter().map(|&i| items[i].clone()).collect()),
            _ => None,
        }
    };
    let normals = get("normals").or_else(|| get("primvars:normals")).and_then(|a| {
        let indices = get("primvars:normals:indices").and_then(|i| i.value.as_ref()).map(|v| v.numbers().into_iter().map(|n| n as usize).collect());
        per_corner(a, 3, indices)
    });
    let uv_name = ["primvars:st", "primvars:UVMap", "primvars:uv", "primvars:st0"]
        .into_iter()
        .find(|n| prim.attributes.contains_key(*n))
        .map(str::to_string)
        .or_else(|| prim.attributes.keys().find(|k| k.starts_with("primvars:") && !k.ends_with(":indices") && prim.attributes[*k].value.as_ref().is_some_and(|v| matches!(v, Val::List(items) if items.first().is_some_and(|i| matches!(i, Val::List(c) if c.len() == 2))))).cloned());
    let uvs = uv_name.and_then(|name| {
        let indices = get(&format!("{name}:indices")).and_then(|i| i.value.as_ref()).map(|v| v.numbers().into_iter().map(|n| n as usize).collect());
        per_corner(get(&name)?, 2, indices)
    });

    // Fanned into triangles, one vertex per corner; the import pipeline
    // shares identical ones again.
    let mut mesh = RawMesh::default();
    let mut corner = 0;
    let flipped = prim.attributes.get("orientation").and_then(|a| a.value.as_ref()).and_then(|v| v.text().map(str::to_string)).as_deref() == Some("leftHanded");
    for &n in &counts {
        if corner + n > corners.len() {
            break;
        }
        for k in 1..n.saturating_sub(1) {
            let tri = if flipped { [corner, corner + k + 1, corner + k] } else { [corner, corner + k, corner + k + 1] };
            for c in tri {
                mesh.indices.push(mesh.positions.len() as u32);
                mesh.positions.push(positions[corners[c]]);
                if let Some(normals) = &normals {
                    let v = &normals[c];
                    mesh.normals.push(vec3(v[0], v[1], v[2]));
                }
                if let Some(uvs) = &uvs {
                    // USD's v runs up the image; Tequila's, down.
                    mesh.uvs.push(vec2(uvs[c][0], 1.0 - uvs[c][1]));
                }
            }
        }
        corner += n;
    }
    if mesh.indices.is_empty() {
        return None;
    }
    mesh.name = prim.name.clone();
    Some(mesh)
}

fn colour(value: &Val) -> Option<Spectrum> {
    let n = value.numbers();
    (n.len() >= 3).then(|| Spectrum::from_linear_srgb(vec3(n[0] as f32, n[1] as f32, n[2] as f32)))
}

/// Where a layer's pictures are: inside its USDZ package, or beside the
/// file, where `import` finds them by the path the layer gives.
struct Images<'a> {
    /// The package's members, and the folder the stage is in within it.
    package: Option<(&'a tequila_format::zip::ZipArchive, String)>,
}

impl Images<'_> {
    /// The picture an asset path names, as the scene carries it.
    fn texture(&self, asset: &str, role: tequila_texture::texture::MapRole) -> Option<RawTexture> {
        let asset = asset.trim();
        if asset.is_empty() {
            return None;
        }
        match &self.package {
            Some((zip, folder)) => {
                // Relative to the stage, `./` and `../` resolved.
                let mut parts: Vec<&str> = folder.split('/').filter(|p| !p.is_empty()).collect();
                for part in asset.trim_start_matches("./").split('/') {
                    match part {
                        "" | "." => {}
                        ".." => {
                            parts.pop();
                        }
                        p => parts.push(p),
                    }
                }
                let member = parts.join("/");
                let bytes = zip.get(&member)?;
                Some(RawTexture { uri: member.replace('/', "_"), role, embedded: Some(std::sync::Arc::new(bytes.to_vec())) })
            }
            None => Some(RawTexture { uri: asset.to_string(), role, embedded: None }),
        }
    }
}

/// The prim and output an input is connected to: `</M/Image.outputs:rgb>`.
fn connected<'a>(shader: &Prim, input: &str, index: &BTreeMap<String, &'a Prim>) -> Option<(&'a Prim, String)> {
    let target = shader.attributes.get(&format!("inputs:{input}.connect"))?.value.as_ref()?;
    let path = match target {
        Val::Path(p) => p.clone(),
        Val::List(items) => items.iter().find_map(|v| if let Val::Path(p) = v { Some(p.clone()) } else { None })?,
        _ => return None,
    };
    let (prim, property) = path.rsplit_once('.')?;
    let output = property.strip_prefix("outputs:").unwrap_or(property).to_string();
    Some((*index.get(prim)?, output))
}

/// A `UsdUVTexture`'s mapping: its wrap, and a `UsdTransform2d` between it
/// and the coordinates when there is one, its scale and offset (turned for
/// Tequila's v running down the image, as the mesh's are on reading).
fn texture_mapping(image: &Prim, index: &BTreeMap<String, &Prim>, warnings: &mut Vec<String>) -> tequila_texture::projection::Mapping {
    use tequila_texture::Wrap;
    let mut mapping = tequila_texture::projection::Mapping::default();
    let wrap = |key: &str| match image.attributes.get(key).and_then(|a| a.value.as_ref()).and_then(|v| v.text()) {
        Some("clamp") | Some("black") => Wrap::Clamp,
        Some("mirror") => Wrap::Mirror,
        _ => Wrap::Repeat,
    };
    mapping.wrap_u = wrap("inputs:wrapS");
    mapping.wrap_v = wrap("inputs:wrapT");
    if let Some((transform, _)) = connected(image, "st", index) {
        let is = |p: &Prim, id: &str| p.attributes.get("info:id").and_then(|a| a.value.as_ref()).and_then(|v| v.text()) == Some(id);
        if is(transform, "UsdTransform2d") {
            let pair = |key: &str, default: f32| -> (f32, f32) {
                let n = transform.attributes.get(key).and_then(|a| a.value.as_ref()).map(|v| v.numbers()).unwrap_or_default();
                if n.len() >= 2 {
                    (n[0] as f32, n[1] as f32)
                } else {
                    (default, default)
                }
            };
            let (su, sv) = pair("inputs:scale", 1.0);
            let (tu, tv) = pair("inputs:translation", 0.0);
            // t' = t * sv + tv, and v = 1 - t: v' = v * sv + (1 - sv - tv).
            mapping.scale = vec3(su, sv, 1.0);
            mapping.offset = vec2(tu, 1.0 - sv - tv);
            let rotation = transform.attributes.get("inputs:rotation").and_then(|a| a.value.as_ref()).map(|v| v.numbers()).unwrap_or_default();
            if rotation.first().is_some_and(|r| r.abs() > 1e-6) {
                warnings.push("a texture's rotation (UsdTransform2d) is not read; its scale and offset are".into());
            }
        }
    }
    mapping
}

/// The images a preview surface's inputs are connected to, as image nodes
/// driving the parameters they stand for.
fn attach_images(preview: &Prim, definition: &mut MaterialDefinition, index: &BTreeMap<String, &Prim>, images: &Images, textures: &mut Vec<RawTexture>, warnings: &mut Vec<String>) {
    use tequila_materials::graph::{Channel, Node, NodeKind, Socket, TextureId, Value};
    use tequila_texture::texture::MapRole;
    enum Kind {
        Colour,
        Scalar,
        Normal,
    }
    for (input, param, kind) in [
        ("diffuseColor", Param::BaseColor, Kind::Colour),
        ("emissiveColor", Param::EmissionColor, Kind::Colour),
        ("roughness", Param::SpecularRoughness, Kind::Scalar),
        ("metallic", Param::BaseMetalness, Kind::Scalar),
        ("opacity", Param::Opacity, Kind::Scalar),
        ("clearcoat", Param::CoatWeight, Kind::Scalar),
        ("clearcoatRoughness", Param::CoatRoughness, Kind::Scalar),
        ("occlusion", Param::Occlusion, Kind::Scalar),
        ("normal", Param::Bump, Kind::Normal),
    ] {
        let Some((image, output)) = connected(preview, input, index) else { continue };
        if image.attributes.get("info:id").and_then(|a| a.value.as_ref()).and_then(|v| v.text()) != Some("UsdUVTexture") {
            continue;
        }
        let Some(file) = image.attributes.get("inputs:file").and_then(|a| a.value.as_ref()).and_then(|v| v.text()).map(str::to_string) else { continue };
        let raw = image.attributes.get("inputs:sourceColorSpace").and_then(|a| a.value.as_ref()).and_then(|v| v.text()) == Some("raw");
        let alpha = output == "a";
        let role = match kind {
            Kind::Normal => MapRole::Normal,
            // The alpha shares the colour image's entry (`sample_alpha`
            // reads the channel the sRGB decode leaves alone).
            _ if alpha => MapRole::Color,
            Kind::Colour if !raw => MapRole::Color,
            _ => MapRole::Data,
        };
        let Some(wanted) = images.texture(&file, role) else {
            warnings.push(format!("the picture {file} is not in the package; the material shows without it"));
            continue;
        };
        let slot = textures.iter().position(|t| t.same_image(&wanted)).unwrap_or_else(|| {
            textures.push(wanted);
            textures.len() - 1
        });
        let mapping = texture_mapping(image, index, warnings);
        let constant = definition.material().clone();
        let graph = &mut definition.graph;
        let output_node = graph.output();
        let coords = graph.add_node(Node::new(NodeKind::Mapping2d { mapping }));
        let texture = TextureId(slot as u32);
        let source = if alpha {
            graph.add_node(Node::new(NodeKind::ImageAlpha { texture, fallback: Value::Float(1.0) }))
        } else {
            let fallback = match (param, &kind) {
                (Param::BaseColor, _) => Value::Color(constant.base_color),
                (Param::EmissionColor, _) => Value::Color(constant.emission_color),
                (_, Kind::Normal) => Value::Float(0.0),
                _ => Value::Float(1.0),
            };
            graph.add_node(Node::new(NodeKind::ImageTexture { texture, fallback }))
        };
        graph.connect(coords, source, Socket::Input(0));
        let channel = match output.as_str() {
            "r" => Some(Channel::Red),
            "g" => Some(Channel::Green),
            "b" => Some(Channel::Blue),
            _ => None,
        };
        let driver = match (channel, &kind) {
            (Some(channel), Kind::Scalar) => {
                let pick = graph.add_node(Node::new(NodeKind::Channel { channel }));
                graph.connect(source, pick, Socket::Input(0));
                pick
            }
            _ => source,
        };
        graph.connect(driver, output_node, Socket::Param(param));
        if param == Param::EmissionColor {
            let m = definition.material_mut();
            if m.emission_luminance <= 0.0 {
                m.emission_color = Spectrum::ONE;
                m.emission_luminance = 100.0;
            }
        }
    }
}

/// A Material prim as a Tequila material: its Tequila definition, its
/// OpenPBR shader's parameters, or its preview surface's with the images
/// it is connected to.
fn material_of(prim: &Prim, index: &BTreeMap<String, &Prim>, images: &Images, textures: &mut Vec<RawTexture>, warnings: &mut Vec<String>) -> MaterialDefinition {
    let name = prim
        .attributes
        .get("tequila:name")
        .and_then(|a| a.value.as_ref())
        .and_then(|v| v.text().map(str::to_string))
        .unwrap_or_else(|| prim.name.clone());
    if let Some(definition) = prim
        .attributes
        .get("tequila:definition")
        .and_then(|a| a.value.as_ref())
        .and_then(|v| v.text())
        .and_then(|json| tequila_format::material_from_json(json).ok())
    {
        return MaterialDefinition { name, ..definition };
    }
    let kind = prim
        .attributes
        .get("tequila:materialType")
        .and_then(|a| a.value.as_ref())
        .and_then(|v| v.text())
        .and_then(MaterialType::from_id);
    let shader = |id: &str| -> Option<&Prim> {
        let mut stack: Vec<&Prim> = prim.children.iter().collect();
        while let Some(p) = stack.pop() {
            if p.attributes.get("info:id").and_then(|a| a.value.as_ref()).and_then(|v| v.text()) == Some(id) {
                return Some(p);
            }
            stack.extend(p.children.iter());
        }
        None
    };
    let input = |p: &Prim, key: &str| p.attributes.get(&format!("inputs:{key}")).and_then(|a| a.value.clone());

    if let Some(pbr) = shader(tequila_format::usd::OPEN_PBR) {
        let mut definition = MaterialDefinition::preset(name, kind.unwrap_or(MaterialType::Generic));
        let m = definition.material_mut();
        for param in Param::ALL {
            let Some(value) = input(pbr, param.key()) else { continue };
            let parsed = match param.read(m) {
                Some(ParamValue::Scalar(_)) => value.numbers().first().map(|v| ParamValue::Scalar(*v as f32)),
                Some(ParamValue::Color(_)) => colour(&value).map(ParamValue::Color),
                Some(ParamValue::Flag(_)) => value.numbers().first().map(|v| ParamValue::Flag(*v != 0.0)),
                _ => None,
            };
            if let Some(parsed) = parsed {
                param.write(m, parsed);
            }
        }
        let n = pbr.attributes.get("tequila:metal_ior_n").and_then(|a| a.value.as_ref()).and_then(colour);
        let k = pbr.attributes.get("tequila:metal_ior_k").and_then(|a| a.value.as_ref()).and_then(colour);
        if let (Some(n), Some(k)) = (n, k) {
            param_metal(m, n, k);
        }
        return definition;
    }

    // A preview surface: the parameters every USD tool writes.
    let mut definition = MaterialDefinition::preset(name, kind.unwrap_or(MaterialType::Plastic));
    if let Some(preview) = shader("UsdPreviewSurface") {
        let m = definition.material_mut();
        let scalar = |key: &str| input(preview, key).and_then(|v| v.numbers().first().map(|n| *n as f32));
        if let Some(c) = input(preview, "diffuseColor").as_ref().and_then(colour) {
            m.base_color = c;
            m.base_weight = 1.0;
        }
        if let Some(v) = scalar("metallic") {
            m.base_metalness = v.clamp(0.0, 1.0);
        }
        if let Some(v) = scalar("roughness") {
            m.specular_roughness = v.clamp(0.0, 1.0);
        }
        if let Some(v) = scalar("ior") {
            m.specular_ior = v.clamp(1.0, 3.0);
        }
        if let Some(v) = scalar("opacity") {
            m.geometry_opacity = v.clamp(0.0, 1.0);
        }
        if let Some(v) = scalar("clearcoat") {
            m.coat_weight = v.clamp(0.0, 1.0);
        }
        if let Some(v) = scalar("clearcoatRoughness") {
            m.coat_roughness = v.clamp(0.0, 1.0);
        }
        if let Some(c) = input(preview, "emissiveColor").as_ref().and_then(colour) {
            if !c.is_black() {
                m.emission_color = c;
                m.emission_luminance = 100.0;
            }
        }
        attach_images(preview, &mut definition, index, images, textures, warnings);
    }
    definition
}

fn param_metal(m: &mut tequila_materials::openpbr::Material, n: Spectrum, k: Spectrum) {
    m.metal_ior = tequila_materials::fresnel::ComplexIor { n, k };
}

// ---- binary layers ----------------------------------------------------------------

/// A binary field's value as the text reader would have read it.
fn crate_value(value: &tequila_format::usdc::Value) -> Val {
    use tequila_format::usdc::Value as V;
    let numbers = |v: &[f64]| Val::List(v.iter().map(|n| Val::Number(*n)).collect());
    let rows = |width: usize, v: Vec<f64>| Val::List(v.chunks(width.max(1)).map(|c| Val::List(c.iter().map(|n| Val::Number(*n)).collect())).collect());
    match value {
        V::Bool(b) => Val::Number(*b as u8 as f64),
        V::Int(i) => Val::Number(*i as f64),
        V::Float(f) => Val::Number(*f as f64),
        V::Double(d) => Val::Number(*d),
        V::Token(t) | V::String(t) | V::Asset(t) => Val::Text(t.clone()),
        V::Vecf(v) => numbers(&v.iter().map(|x| *x as f64).collect::<Vec<_>>()),
        V::Vecd(v) | V::Doubles(v) => numbers(v),
        V::Ints(v) => Val::List(v.iter().map(|n| Val::Number(*n as f64)).collect()),
        V::Floats(v) => Val::List(v.iter().map(|n| Val::Number(*n as f64)).collect()),
        V::Vecfs(width, v) => rows(*width, v.iter().map(|x| *x as f64).collect()),
        V::Vecds(width, v) => rows(*width, v.clone()),
        V::Tokens(v) | V::Strings(v) | V::TokenVector(v) => Val::List(v.iter().map(|t| Val::Text(t.clone())).collect()),
        V::TokenList(op) => Val::List(op.items.iter().map(|t| Val::Text(t.clone())).collect()),
        V::Paths(op) => match op.items.as_slice() {
            [one] => Val::Path(one.clone()),
            many => Val::List(many.iter().map(|p| Val::Path(p.clone())).collect()),
        },
        // Only references within the file are followed.
        V::References(op) => Val::List(op.items.iter().filter(|r| r.asset.is_empty()).map(|r| Val::Path(r.prim.clone())).collect()),
        V::Specifier(_) | V::Variability(_) | V::Other(_) => Val::None,
    }
}

/// A binary layer as the prims the text reader makes: the stage's metadata
/// and the pseudo-root.
fn prims_from_crate(layer: &tequila_format::usdc::Layer) -> (BTreeMap<String, Val>, Prim) {
    use tequila_format::usdc::{Spec, Specifier, Value as V};
    let specs: BTreeMap<&str, &Spec> = layer.specs.iter().map(|s| (s.path.as_str(), s)).collect();
    fn names(spec: &Spec, field: &str) -> Vec<String> {
        match spec.field(field) {
            Some(V::TokenVector(v)) => v.clone(),
            _ => Vec::new(),
        }
    }
    fn build(path: &str, name: &str, specs: &BTreeMap<&str, &Spec>, depth: usize) -> Prim {
        let mut prim = Prim { name: name.to_string(), specifier: "def".into(), ..Prim::default() };
        let Some(spec) = specs.get(path) else { return prim };
        for (field, value) in &spec.fields {
            match (field.as_str(), value) {
                ("typeName", V::Token(t)) => prim.kind = t.clone(),
                ("specifier", V::Specifier(s)) => {
                    prim.specifier = match s {
                        Specifier::Def => "def",
                        Specifier::Over => "over",
                        Specifier::Class => "class",
                    }
                    .into()
                }
                ("primChildren" | "properties", _) => {}
                (other, value) => {
                    prim.meta.insert(other.to_string(), crate_value(value));
                }
            }
        }
        for property in names(spec, "properties") {
            let Some(p) = specs.get(format!("{path}.{property}").as_str()) else { continue };
            let mut attribute = Attribute::default();
            for (field, value) in &p.fields {
                match field.as_str() {
                    "default" | "targetPaths" => attribute.value = Some(crate_value(value)),
                    // Written as the text form writes it: `name.connect`.
                    "connectionPaths" => {
                        prim.attributes.insert(format!("{property}.connect"), Attribute { value: Some(crate_value(value)), meta: BTreeMap::new() });
                    }
                    "typeName" | "variability" | "custom" => {}
                    other => {
                        attribute.meta.insert(other.to_string(), crate_value(value));
                    }
                }
            }
            prim.attributes.insert(property, attribute);
        }
        if depth < DEEPEST {
            for child in names(spec, "primChildren") {
                let child_path = if path == "/" { format!("/{child}") } else { format!("{path}/{child}") };
                prim.children.push(build(&child_path, &child, specs, depth + 1));
            }
        }
        prim
    }
    let mut stage = BTreeMap::new();
    if let Some(root) = specs.get("/") {
        for (field, value) in &root.fields {
            if field != "primChildren" {
                stage.insert(field.clone(), crate_value(value));
            }
        }
    }
    let root = build("/", "", &specs, 0);
    (stage, root)
}

/// A layer, text or binary, as its stage's metadata and its prims.
fn parse(bytes: &[u8]) -> Result<(BTreeMap<String, Val>, Prim), ImportError> {
    if bytes.starts_with(b"PXR-USDC") {
        let layer = tequila_format::usdc::read(bytes).map_err(|e| ImportError::Malformed(format!("this binary USD could not be read: {e}")))?;
        return Ok(prims_from_crate(&layer));
    }
    let text = String::from_utf8_lossy(bytes);
    let tokens = tokenize(&text).map_err(|e| ImportError::Malformed(format!("this is not USD text Tequila can read: {e}")))?;
    let mut parser = Parser { tokens, at: 0, depth: 0 };
    // The header: `#usda 1.0` is a comment to the tokenizer; then the
    // stage's metadata.
    let mut stage = BTreeMap::new();
    if parser.eat('(') {
        stage = parser.metadata();
    }
    let mut root = Prim::default();
    parser.body(&mut root);
    Ok((stage, root))
}

/// Read a `.usda` or `.usdc` layer. Its pictures are found beside it.
pub fn read(bytes: &[u8], name: &str) -> Result<RawScene, ImportError> {
    let (stage, root) = parse(bytes)?;
    scene_of(stage, root, name, &Images { package: None })
}

/// Read a `.usdz` package: its first file is the stage, text or binary,
/// and its pictures are inside it.
pub fn read_package(bytes: &[u8], name: &str) -> Result<RawScene, ImportError> {
    let zip = tequila_format::zip::ZipArchive::read(bytes).map_err(|e| ImportError::Malformed(format!("this USDZ package could not be opened: {e}")))?;
    let first = zip.names().first().cloned().ok_or_else(|| ImportError::Malformed("this USDZ package is empty".into()))?;
    let extension = first.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if !matches!(extension.as_str(), "usd" | "usda" | "usdc") {
        return Err(ImportError::Malformed(format!("this USDZ package starts with {first}, not a USD stage")));
    }
    let (stage, root) = parse(zip.get(&first).unwrap_or(&[]))?;
    let folder = first.rsplit_once('/').map(|(f, _)| f.to_string()).unwrap_or_default();
    scene_of(stage, root, name, &Images { package: Some((&zip, folder)) })
}

/// The prims a prim references within the file.
fn references(prim: &Prim) -> Vec<String> {
    match prim.meta.get("references") {
        Some(Val::Path(p)) => vec![p.clone()],
        Some(Val::List(items)) => items.iter().filter_map(|v| if let Val::Path(p) = v { Some(p.clone()) } else { None }).collect(),
        _ => Vec::new(),
    }
}

/// A prim with what it references composed under its own opinions: the
/// referenced prim's attributes where it has none of its own, and its
/// children beside its own. USD's instancing - an instanceable prim
/// referencing a prototype - comes in as a copy at each place.
fn composed(prim: &Prim, index: &BTreeMap<String, &Prim>, depth: usize) -> Prim {
    let mut out = prim.clone();
    out.meta.remove("references");
    if depth >= 16 {
        return out;
    }
    for target in references(prim) {
        let Some(source) = index.get(&target) else { continue };
        let source = composed(source, index, depth + 1);
        if out.kind.is_empty() {
            out.kind = source.kind.clone();
        }
        for (key, attribute) in source.attributes {
            out.attributes.entry(key).or_insert(attribute);
        }
        for child in source.children {
            if !out.children.iter().any(|c| c.name == child.name) {
                out.children.push(child);
            }
        }
    }
    out
}

/// The scene a layer's prims describe.
fn scene_of(stage: BTreeMap<String, Val>, root: Prim, name: &str, images: &Images) -> Result<RawScene, ImportError> {
    let mut scene = RawScene { source_unit: None, ..RawScene::default() };
    scene.source_unit = stage.get("metersPerUnit").and_then(|v| v.numbers().first().copied()).map(|m| {
        [(0.001, LengthUnit::Millimetre), (0.01, LengthUnit::Centimetre), (0.0254, LengthUnit::Inch), (0.3048, LengthUnit::Foot), (1.0, LengthUnit::Metre)]
            .into_iter()
            .min_by(|a, b| (a.0 - m).abs().total_cmp(&(b.0 - m).abs()))
            .map(|(_, u)| u)
            .unwrap_or(LengthUnit::Metre)
    });
    let z_up = stage.get("upAxis").and_then(|v| v.text().map(str::to_string)).as_deref() == Some("Z");

    // Every prim by its path, for connections and references.
    fn index_of<'a>(prim: &'a Prim, path: &str, out: &mut BTreeMap<String, &'a Prim>) {
        for child in &prim.children {
            let here = format!("{path}/{}", child.name);
            out.insert(here.clone(), child);
            index_of(child, &here, out);
        }
    }
    let mut index: BTreeMap<String, &Prim> = BTreeMap::new();
    index_of(&root, "", &mut index);

    // Materials anywhere in the stage, by path.
    let mut materials: BTreeMap<String, usize> = BTreeMap::new();
    fn collect<'a>(prim: &'a Prim, path: &str, out: &mut Vec<(String, &'a Prim)>) {
        for child in &prim.children {
            let here = format!("{path}/{}", child.name);
            if child.kind == "Material" {
                out.push((here.clone(), child));
            }
            collect(child, &here, out);
        }
    }
    let mut found = Vec::new();
    collect(&root, "", &mut found);
    for (path, prim) in found {
        materials.insert(path, scene.materials.len());
        let material = material_of(prim, &index, images, &mut scene.textures, &mut scene.warnings);
        scene.materials.push(material);
    }

    // The hierarchy.
    let top = if z_up {
        // Y up: (x, y, z) → (x, z, -y), around a group of the stage's own.
        let mut turn = Mat4::IDENTITY;
        turn.m[1][1] = 0.0;
        turn.m[1][2] = 1.0;
        turn.m[2][1] = -1.0;
        turn.m[2][2] = 0.0;
        let mut node = RawNode::group(name);
        node.transform = Transform::from_matrix(turn).unwrap_or(Transform::IDENTITY);
        scene.nodes.push(node);
        scene.roots.push(0);
        Some(0)
    } else {
        None
    };
    #[allow(clippy::too_many_arguments)]
    fn walk(prim: &Prim, path: &str, parent: Option<usize>, binding: Option<&str>, materials: &BTreeMap<String, usize>, index: &BTreeMap<String, &Prim>, scene: &mut RawScene) {
        // A class is a prototype for others, not drawn itself.
        if prim.specifier == "class" {
            return;
        }
        let composed_prim;
        let prim = if references(prim).is_empty() {
            prim
        } else {
            composed_prim = composed(prim, index, 0);
            &composed_prim
        };
        let skip = matches!(prim.kind.as_str(), "Material" | "Shader" | "Scope" | "GeomSubset" | "NodeGraph") || prim.kind.ends_with("Light") || prim.kind == "Camera";
        if skip && prim.kind != "Scope" {
            return;
        }
        let bound = prim
            .attributes
            .get("material:binding")
            .and_then(|a| a.value.as_ref())
            .and_then(|v| match v {
                Val::Path(p) => Some(p.as_str()),
                Val::List(items) => items.iter().find_map(|i| if let Val::Path(p) = i { Some(p.as_str()) } else { None }),
                _ => None,
            })
            .or(binding);
        let visible = prim.attributes.get("visibility").and_then(|a| a.value.as_ref()).and_then(|v| v.text()) != Some("invisible");
        let (node_index, own_parent) = if prim.kind == "Scope" {
            (parent, parent)
        } else {
            let mut node = match (prim.kind.as_str(), mesh_of(prim)) {
                ("Mesh", Some(mut mesh)) => {
                    mesh.material = bound.and_then(|b| materials.get(b).copied());
                    scene.meshes.push(mesh);
                    RawNode::part(prim.name.clone(), scene.meshes.len() - 1)
                }
                _ => RawNode::group(prim.name.clone()),
            };
            node.parent = parent;
            node.transform = transform_of(prim);
            if !visible {
                node.metadata.insert("USD visibility".into(), "invisible".into());
            }
            scene.nodes.push(node);
            let index = scene.nodes.len() - 1;
            if parent.is_none() {
                scene.roots.push(index);
            }
            (Some(index), Some(index))
        };
        let _ = own_parent;
        for child in &prim.children {
            walk(child, &format!("{path}/{}", child.name), node_index, bound, materials, index, scene);
        }
    }
    for prim in &root.children {
        walk(prim, &format!("/{}", prim.name), top, None, &materials, &index, &mut scene);
    }
    if scene.meshes.is_empty() {
        return Err(ImportError::Unsupported("the USD file has no meshes Tequila can read".into()));
    }
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_words_and_strings_tokenize() {
        let t = tokenize("float3 x = (1.5, -2e-3, +4) \"a \\\"b\\\"\" <Scene/M> @tex.png@ # comment").unwrap();
        assert_eq!(t[0], Token::Word("float3".into()));
        assert!(t.contains(&Token::Number(-2e-3)));
        assert!(t.contains(&Token::Number(4.0)));
        assert!(t.contains(&Token::Text("a \"b\"".into())));
        assert!(t.contains(&Token::Path("Scene/M".into())));
        assert!(t.contains(&Token::Asset("tex.png".into())));
    }

    /// A quad from Blender's exporter: Z up, translate and rotateXYZ, UVs
    /// face-varying and indexed.
    #[test]
    fn a_blender_style_quad_reads() {
        let text = r#"#usda 1.0
(
    defaultPrim = "root"
    metersPerUnit = 1
    upAxis = "Z"
)
def Xform "root"
{
    def Xform "Plane"
    {
        float3 xformOp:rotateXYZ = (0, 0, 90)
        double3 xformOp:translate = (1, 0, 0)
        uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ"]
        def Mesh "Plane" (
            prepend apiSchemas = ["MaterialBindingAPI"]
        )
        {
            int[] faceVertexCounts = [4]
            int[] faceVertexIndices = [0, 1, 3, 2]
            normal3f[] normals = [(0, 0, 1), (0, 0, 1), (0, 0, 1), (0, 0, 1)] (
                interpolation = "faceVarying"
            )
            point3f[] points = [(-1, -1, 0), (1, -1, 0), (-1, 1, 0), (1, 1, 0)]
            texCoord2f[] primvars:UVMap = [(0, 0), (1, 0), (1, 1), (0, 1)] (
                interpolation = "faceVarying"
            )
            int[] primvars:UVMap:indices = [0, 1, 2, 3]
            rel material:binding = </root/_materials/Red>
        }
    }
    def Scope "_materials"
    {
        def Material "Red"
        {
            token outputs:surface.connect = </root/_materials/Red/Principled_BSDF.outputs:surface>
            def Shader "Principled_BSDF"
            {
                uniform token info:id = "UsdPreviewSurface"
                color3f inputs:diffuseColor = (0.8, 0.1, 0.05)
                float inputs:roughness = 0.4
                token outputs:surface
            }
        }
    }
}
"#;
        let scene = read(text.as_bytes(), "plane").expect("reads");
        assert_eq!(scene.meshes.len(), 1);
        let mesh = &scene.meshes[0];
        assert_eq!(mesh.indices.len(), 6, "a quad is two triangles");
        assert_eq!(mesh.uvs[1], vec2(1.0, 1.0), "v flipped");
        let material = &scene.materials[mesh.material.expect("bound")];
        assert!((material.material().specular_roughness - 0.4).abs() < 1e-6);
        // The stage turned Y up, the plane's group moved along X.
        let plane_group = scene.nodes.iter().find(|n| n.name == "Plane" && n.mesh.is_none()).unwrap();
        assert!((plane_group.transform.point(Vec3::ZERO) - vec3(1.0, 0.0, 0.0)).length() < 1e-5);
        let top = &scene.nodes[scene.roots[0]];
        assert!((top.transform.vector(vec3(0.0, 0.0, 1.0)) - vec3(0.0, 1.0, 0.0)).length() < 1e-5, "Z up turned to Y up");
    }

    /// A binary file cut short is refused as what it is, not read as text.
    #[test]
    fn a_truncated_binary_usd_is_refused_by_name() {
        let error = read(b"PXR-USDC\0\0\0\0", "x").err().expect("refused");
        assert!(error.to_string().contains("binary USD"), "{error}");
    }

    /// Instancing as USD writes it: a prototype under a class, referenced
    /// by two instanceable prims, comes in as a part at each, wearing the
    /// material each binds; the class itself is not drawn.
    #[test]
    fn internal_references_are_followed_and_classes_not_drawn() {
        let text = r#"#usda 1.0
(
    defaultPrim = "Product"
    metersPerUnit = 1
    upAxis = "Y"
)
def Xform "Product"
{
    def Xform "Left" (
        instanceable = true
        prepend references = </Prototypes/Bolt>
    )
    {
        rel material:binding = </Product/Looks/Red>
        double3 xformOp:translate = (-1, 0, 0)
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }
    def Xform "Right" (
        instanceable = true
        prepend references = </Prototypes/Bolt>
    )
    {
        double3 xformOp:translate = (1, 0, 0)
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }
    def Scope "Looks"
    {
        def Material "Red"
        {
            def Shader "Surface"
            {
                uniform token info:id = "UsdPreviewSurface"
                color3f inputs:diffuseColor = (0.8, 0.1, 0.05)
            }
        }
    }
}
class "Prototypes"
{
    def Xform "Bolt"
    {
        def Mesh "Mesh"
        {
            int[] faceVertexCounts = [3]
            int[] faceVertexIndices = [0, 1, 2]
            point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        }
    }
}
"#;
        let scene = read(text.as_bytes(), "bolts").expect("reads");
        assert_eq!(scene.meshes.len(), 2, "a mesh at each instance, none for the class");
        assert_eq!(scene.meshes.iter().filter(|m| m.material.is_some()).count(), 1, "the left one binds red");
        let left = scene.nodes.iter().find(|n| n.name == "Left").unwrap();
        assert!((left.transform.point(Vec3::ZERO) - vec3(-1.0, 0.0, 0.0)).length() < 1e-6);
        assert!(!scene.nodes.iter().any(|n| n.name == "Prototypes"));
    }
}
