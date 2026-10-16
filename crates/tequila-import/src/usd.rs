//! USD import, from `.usda` text (`docs/03` §6.3).
//!
//! The text form of USD, as Tequila writes it and as Blender, Houdini and
//! usdcat write it: prims and their typed attributes, relationships, and
//! metadata, with nothing it does not need parsed beyond skipping it. The
//! binary `.usdc` form is not read; `usdcat file.usdc -o file.usda` makes
//! one of the other.
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
//!   OpenPBR MaterialX shader, or a `UsdPreviewSurface`'s.
//! * `upAxis = "Z"` turns the stage Y up; `metersPerUnit` is the source's
//!   unit.
//!
//! References, payloads, variants and composition arcs are not followed:
//! what is in the file is what is read.

use crate::raw::{RawMesh, RawNode, RawScene};
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
    kind: String,
    name: String,
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
                    if let Some(child) = self.prim() {
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
            self.metadata();
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

/// A Material prim as a Tequila material: its Tequila definition, its
/// OpenPBR shader's parameters, or its preview surface's.
fn material_of(prim: &Prim) -> MaterialDefinition {
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
    }
    definition
}

fn param_metal(m: &mut tequila_materials::openpbr::Material, n: Spectrum, k: Spectrum) {
    m.metal_ior = tequila_materials::fresnel::ComplexIor { n, k };
}

/// Read a `.usda` file.
pub fn read(bytes: &[u8], name: &str) -> Result<RawScene, ImportError> {
    let text = String::from_utf8_lossy(bytes);
    if text.starts_with("PXR-USDC") {
        return Err(ImportError::Unsupported(
            "this is binary USD (.usdc); Tequila reads the text form - `usdcat file.usdc -o file.usda` writes one".into(),
        ));
    }
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

    let mut scene = RawScene { source_unit: None, ..RawScene::default() };
    scene.source_unit = stage.get("metersPerUnit").and_then(|v| v.numbers().first().copied()).map(|m| {
        [(0.001, LengthUnit::Millimetre), (0.01, LengthUnit::Centimetre), (0.0254, LengthUnit::Inch), (0.3048, LengthUnit::Foot), (1.0, LengthUnit::Metre)]
            .into_iter()
            .min_by(|a, b| (a.0 - m).abs().total_cmp(&(b.0 - m).abs()))
            .map(|(_, u)| u)
            .unwrap_or(LengthUnit::Metre)
    });
    let z_up = stage.get("upAxis").and_then(|v| v.text().map(str::to_string)).as_deref() == Some("Z");

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
        scene.materials.push(material_of(prim));
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
    fn walk(prim: &Prim, path: &str, parent: Option<usize>, binding: Option<&str>, materials: &BTreeMap<String, usize>, scene: &mut RawScene) {
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
        let (index, own_parent) = if prim.kind == "Scope" {
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
            walk(child, &format!("{path}/{}", child.name), index, bound, materials, scene);
        }
    }
    for prim in &root.children {
        walk(prim, &format!("/{}", prim.name), top, None, &materials, &mut scene);
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

    #[test]
    fn binary_usd_is_named_not_misread() {
        let error = read(b"PXR-USDC\0\0\0\0", "x").err().expect("refused");
        assert!(error.to_string().contains("usdcat"), "{error}");
    }
}
