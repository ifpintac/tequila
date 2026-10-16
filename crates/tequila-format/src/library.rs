//! The material library: presets on disk (`docs/04-UX-SPEC.md` §9).
//!
//! A preset is a *recipe*, and that is why it is stored differently from a
//! material inside a document. A preset records only what it changes from its
//! catalog type's defaults; a document records every parameter explicitly.
//!
//! The two choices look inconsistent and are not. A document has to mean the
//! same thing forever — retuning the Plastic preset in a later release must not
//! silently alter someone's saved scene — so it carries its own values. A
//! library preset is the opposite: if Plastic is retuned, every library plastic
//! *should* follow, or the library is fifty frozen copies rather than a library.
//!
//! Nothing here is traced from a commercial swatch system. `TEQUILA_SPEC.md`
//! §0.2 rule 3 forbids bundling Pantone, RAL, Mold-Tech, Sorensen, Coloro,
//! INTERPON or Axalta libraries, so the names describe materials rather than
//! naming anyone's catalogue entries, and the values come from published
//! physical data and ordinary observation.

use crate::json;
use crate::FormatError;
use tequila_materials::fresnel::metals;
use tequila_materials::material_type::{MaterialType, Param, ParamUi, ParamValue};
use tequila_materials::graph::MaterialGraph;
use tequila_materials::openpbr::Material;
use tequila_materials::graph::TextureId;
use tequila_scene::material::MaterialDefinition;
use tequila_scene::textures::{TextureSource, TextureStore};
use std::path::{Path, PathBuf};

/// Where a library entry came from, and therefore what may be done to it.
///
/// The distinction is not cosmetic. The shipped library is part of the
/// installation: it is the same for everybody, it is what the documentation
/// describes, and a person who edits it has silently changed what "ABS Black"
/// means in every project they ever open. So it is read-only, and saving a
/// modified copy writes a *new* entry into the user's own directory instead.
///
/// The rule that follows is worth stating because it is the whole of the
/// design: nothing in the application ever writes to the standard library, and
/// nothing outside the user's directory can be renamed or deleted from inside
/// the application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Shipped with the application. Read-only.
    Standard,
    /// The user's own, in their configuration directory. Writable.
    User,
    /// Brought by an enabled plugin. Read-only, as the shipped ones are.
    Plugin,
}

/// A library entry: the material, plus what the browser needs to find it.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryEntry {
    pub name: String,
    /// The group it appears under: Plastic, Metal, Paint, Glass, Fabric, Other.
    pub category: String,
    /// Words to match when searching, beyond the name and category.
    pub tags: Vec<String>,
    pub material_type: MaterialType,
    pub material: Material,
    /// The material as a graph: patterns driving its parameters, when it has
    /// any, over a surface node holding `material`. A plain preset's graph is
    /// that surface node alone.
    pub graph: MaterialGraph,
    /// The pictures the graph reads, by the ids its nodes carry: numbered
    /// from one within the entry, and renumbered into a document's own ids
    /// when the entry is applied ([`LibraryEntry::for_document`]). A saved
    /// entry's pictures are copies kept beside it in the library, so the
    /// entry does not break when the project it came from moves.
    pub textures: Vec<(TextureId, TextureSource)>,
    pub source: Source,
    /// The file it was read from, so it can be rewritten or removed. Empty for
    /// an entry that has never been on disk.
    pub path: String,
}

impl LibraryEntry {
    /// Whether the application may rewrite or remove this entry.
    pub fn is_editable(&self) -> bool {
        self.source == Source::User && !self.path.is_empty()
    }

    pub fn definition(&self) -> MaterialDefinition {
        let mut definition = MaterialDefinition::new(
            self.name.clone(),
            self.material_type,
            self.material.clone(),
        );
        definition.graph = self.graph.clone();
        definition
    }

    /// The material as `document` would hold it, and the textures it must
    /// add first.
    ///
    /// A picture the document already has — the same file in the same role —
    /// is reused rather than added again, so applying an entry twice gives
    /// the same graph both times and the caller's check for a material that
    /// is already there still finds it.
    pub fn for_document(
        &self,
        document: &tequila_scene::SceneDocument,
    ) -> (MaterialDefinition, Vec<(TextureId, TextureSource)>) {
        let mut definition = self.definition();
        if self.textures.is_empty() {
            return (definition, Vec::new());
        }
        let mut next = document.textures.next_id();
        let mut renumbered = std::collections::BTreeMap::new();
        let mut added = Vec::new();
        for (id, source) in &self.textures {
            let existing = document
                .textures
                .iter()
                .find(|(_, s)| s.path == source.path && s.role == source.role)
                .map(|(id, _)| id);
            let target = existing.unwrap_or_else(|| {
                let target = TextureId(next);
                next += 1;
                added.push((target, source.clone()));
                target
            });
            renumbered.insert(*id, target);
        }
        definition
            .graph
            .remap_textures(&|id| renumbered.get(&id).copied().unwrap_or(id));
        (definition, added)
    }

    /// An entry for a material that keeps its pictures: the graph as it
    /// stands, and the files `textures` says it reads, renumbered from one.
    /// [`save_entry`] copies the files into the library.
    ///
    /// Falls back to [`LibraryEntry::from_graph`] — surface and patterns
    /// only — when a picture's file cannot be found, or the graph reads a
    /// volume, which is a project's data rather than a material's.
    pub fn from_graph_with_pictures(
        name: String,
        category: String,
        material_type: MaterialType,
        graph: &MaterialGraph,
        textures: &TextureStore,
        source: Source,
    ) -> LibraryEntry {
        use tequila_materials::graph::NodeKind;
        let plain = LibraryEntry::from_graph(name, category, material_type, graph, source);
        let mut ids = std::collections::BTreeSet::new();
        for (_, node) in graph.nodes() {
            match node.kind {
                NodeKind::ImageTexture { texture, .. } | NodeKind::ImageAlpha { texture, .. } => {
                    ids.insert(texture);
                }
                NodeKind::VolumeMap { .. } => return plain,
                _ => {}
            }
        }
        if ids.is_empty() {
            return plain;
        }
        let mut renumbered = std::collections::BTreeMap::new();
        let mut kept = Vec::new();
        for (index, id) in ids.iter().enumerate() {
            let Some(found) = textures.get(*id).filter(|s| s.path.is_file()) else {
                return plain;
            };
            let local = TextureId(index as u32 + 1);
            renumbered.insert(*id, local);
            kept.push((local, found.clone()));
        }
        let mut kept_graph = graph.clone();
        kept_graph.decals.clear();
        kept_graph.remap_textures(&|id| renumbered.get(&id).copied().unwrap_or(id));
        LibraryEntry { graph: kept_graph, textures: kept, ..plain }
    }

    /// An entry for a material, keeping the parts of its graph a library can
    /// hold. Patterns go with it; pictures do not, since a texture or a label
    /// belongs to the project whose files it names, so a graph that uses one
    /// is saved as its surface parameters alone.
    pub fn from_graph(
        name: String,
        category: String,
        material_type: MaterialType,
        graph: &MaterialGraph,
        source: Source,
    ) -> LibraryEntry {
        use tequila_materials::graph::NodeKind;
        let material = graph.material().clone();
        let pictures = graph.nodes().any(|(_, node)| {
            // Files the project refers to: pictures, and volumes.
            matches!(
                node.kind,
                NodeKind::ImageTexture { .. } | NodeKind::ImageAlpha { .. } | NodeKind::VolumeMap { .. }
            )
        });
        let graph = if pictures {
            MaterialGraph::from_material(material.clone())
        } else {
            let mut kept = graph.clone();
            kept.decals.clear();
            kept
        };
        LibraryEntry {
            name,
            category,
            tags: Vec::new(),
            material_type,
            material,
            graph,
            textures: Vec::new(),
            source,
            path: String::new(),
        }
    }

    /// Whether a search query matches. Case-insensitive, and matches any word
    /// of the query against any of the name, category or tags — so "brushed
    /// metal" finds Aluminium Brushed even though neither word is its name.
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        let haystack = format!(
            "{} {} {}",
            self.name.to_lowercase(),
            self.category.to_lowercase(),
            self.tags.join(" ").to_lowercase()
        );
        query.split_whitespace().all(|word| haystack.contains(word))
    }
}

/// Read one preset file.
pub fn read_entry(text: &str, name_hint: &str) -> Result<LibraryEntry, FormatError> {
    let value = json::parse(text.as_bytes())
        .map_err(|e| FormatError::Invalid(format!("{name_hint}: {e}")))?;

    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(name_hint)
        .to_string();
    let type_text = value
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| FormatError::Invalid(format!("{name_hint}: no type")))?;
    let material_type = MaterialType::from_id(type_text).ok_or_else(|| {
        FormatError::Invalid(format!("{name_hint}: `{type_text}` is not a material type"))
    })?;

    // The recipe: start from the type, apply what the file changes.
    let mut material = material_type.defaults();

    // A metal is named rather than given as a complex index of refraction —
    // nobody hand-writes n and k, and the measured table is the point of
    // shipping measured data at all.
    if let Some(named) = value.get("metal_preset").and_then(|v| v.as_str()) {
        let found = metals::ALL
            .iter()
            .find(|(label, _)| label.eq_ignore_ascii_case(named));
        match found {
            Some((_, ior)) => material.metal_ior = *ior,
            None => {
                return Err(FormatError::Invalid(format!(
                    "{name_hint}: `{named}` is not a metal in the measured table"
                )))
            }
        }
    }

    for param in Param::ALL {
        let Some(present) = value.get(param.key()) else {
            continue;
        };
        let Some(current) = param.read(&material) else {
            continue;
        };
        let parsed = match current {
            ParamValue::Scalar(_) => present
                .as_f32()
                .filter(|v| v.is_finite())
                .map(ParamValue::Scalar),
            ParamValue::Color(_) => crate::read_spectrum(present).map(ParamValue::Color),
            ParamValue::Flag(_) => present.as_bool().map(ParamValue::Flag),
            // Handled above by name.
            ParamValue::Metal(_) => None,
        };
        let Some(parsed) = parsed else {
            return Err(FormatError::Invalid(format!(
                "{name_hint}: `{}` has a value of the wrong shape",
                param.key()
            )));
        };
        param.write(&mut material, parsed);
    }
    let material = material.sanitized();

    // The patterns: a short recipe of which parameter a procedural drives
    // and how, built into the graph the way the Material tab builds it. A
    // saved graph is read as it stands.
    let graph = match (value.get("patterns"), value.get("graph")) {
        (Some(patterns), _) => {
            let mut graph = MaterialGraph::from_material(material.clone());
            for pattern in patterns.as_array() {
                apply_pattern(&mut graph, pattern, name_hint)?;
            }
            graph
        }
        (None, Some(saved)) => crate::graph::read_graph(Some(saved), material.clone(), name_hint)?,
        (None, None) => MaterialGraph::from_material(material.clone()),
    };
    graph
        .compile()
        .map_err(|e| FormatError::Invalid(format!("{name_hint}: the patterns do not compile: {e:?}")))?;

    // Pictures, by path: relative ones are relative to the entry's own file,
    // and [`read_directory_from`] resolves them once it knows where that is.
    let mut textures = Vec::new();
    for texture in value.get("textures").map(|v| v.as_array()).unwrap_or(&[]) {
        let (Some(id), Some(path)) = (
            texture.get("id").and_then(|v| v.as_index()),
            texture.get("path").and_then(|v| v.as_str()),
        ) else {
            return Err(FormatError::Invalid(format!("{name_hint}: a texture has no id or no path")));
        };
        let role = texture
            .get("role")
            .and_then(|v| v.as_str())
            .and_then(crate::journal::role_from_name)
            .unwrap_or(tequila_texture::texture::MapRole::Color);
        textures.push((
            TextureId(id as u32),
            TextureSource {
                path: PathBuf::from(path),
                role,
                color_space: texture
                    .get("color_space")
                    .and_then(|v| v.as_str())
                    .and_then(crate::journal::color_space_from_name),
            },
        ));
    }

    Ok(LibraryEntry {
        name,
        category: value
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("Other")
            .to_string(),
        tags: value
            .get("tags")
            .map(|v| v.as_array())
            .unwrap_or(&[])
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        material_type,
        material,
        graph,
        textures,
        // Filled in by whoever knows where the file came from.
        source: Source::Standard,
        path: String::new(),
    })
}

/// One entry of a preset's `patterns`:
///
/// ```json
/// {"param": "base_color", "procedural": {"kind": "weave", ...}, "seed": 3,
///  "colors": [{"at": 0, "color": [r, g, b]}, {"at": 1, "color": [r, g, b]}]}
/// {"param": "specular_roughness", "procedural": {...}, "range": [0.5, 0.8]}
/// {"param": "bump", "procedural": {...}, "depth": 0.0002}
/// ```
///
/// `colors` is the ramp a colour is read through, `range` what a number's 0
/// to 1 becomes, and `depth` how high a bump's 1 stands, in metres. `size`,
/// in metres, is the pattern's feature size, for a procedural that gives no
/// `frequency` (which is repeats per metre: `size` 0.01 is `frequency` 100).
///
/// A pattern on a parameter something already drives - or one with
/// `blend`, `amount` or `mask` - is a layer over what is there: the earlier
/// pattern, or the parameter's own value. `blend` is `mix` (the default),
/// `multiply` or `add`; `amount`, 0 to 1, how much of it shows; `mask`
/// where it shows - `{"threshold": 0.6, "softness": 0.05}` cuts this
/// pattern's own value at 0.6, softly, and a `procedural` in the mask cuts
/// another pattern instead; `"invert": true` turns it inside out. Speckles
/// over a base, veins through stone, dirt in a weave.
fn apply_pattern(graph: &mut MaterialGraph, pattern: &json::Value, name: &str) -> Result<(), FormatError> {
    use tequila_core::math::vec2;
    use tequila_materials::graph::{Node, NodeKind, Socket};

    let invalid = |what: &str| FormatError::Invalid(format!("{name}: a pattern {what}"));
    let key = pattern.get("param").and_then(|v| v.as_str()).ok_or_else(|| invalid("names no parameter"))?;
    let param = Param::from_key(key).ok_or_else(|| invalid(&format!("names `{key}`, which is no parameter")))?;
    let procedural = pattern_procedural(pattern.get("procedural"), pattern.get("size")).ok_or_else(|| invalid("has no procedural this version knows"))?;
    let seed = pattern.get("seed").and_then(|v| v.as_f32()).unwrap_or(0.0) as u32;

    let layered = graph.driver_of(param).is_some()
        || pattern.get("blend").is_some()
        || pattern.get("amount").is_some()
        || pattern.get("mask").is_some();
    if layered {
        return layer_pattern(graph, param, procedural, seed, pattern, name);
    }

    tequila_scene::command::attach_procedural(graph, param, procedural, seed);

    if let Some(colors) = pattern.get("colors") {
        let gradient = crate::graph::gradient_from_json(colors).ok_or_else(|| invalid("has an unreadable colour ramp"))?;
        if !tequila_scene::command::set_gradient(graph, param, gradient) {
            return Err(invalid(&format!("gives colours to `{key}`, which is not a colour")));
        }
    }
    if let Some(range) = pattern.get("range").map(|v| v.as_array()) {
        let (Some(low), Some(high)) = (range.first().and_then(|v| v.as_f32()), range.get(1).and_then(|v| v.as_f32())) else {
            return Err(invalid("has a range that is not two numbers"));
        };
        let driver = graph.driver_of(param).ok_or_else(|| invalid("drives nothing"))?;
        let remap = graph.add_node(Node::new(NodeKind::Remap { from: vec2(0.0, 1.0), to: vec2(low, high) }));
        let output = graph.output();
        graph.connect(driver, remap, Socket::Input(0));
        graph.connect(remap, output, Socket::Param(param));
    }
    if let Some(depth) = pattern.get("depth").and_then(|v| v.as_f32()) {
        let driver = graph.driver_of(param).ok_or_else(|| invalid("drives nothing"))?;
        match graph.node_mut(driver).map(|n| &mut n.kind) {
            Some(NodeKind::Bump { depth: d }) => *d = depth,
            _ => return Err(invalid(&format!("gives a depth to `{key}`, which is not a bump"))),
        }
    }
    Ok(())
}

/// A pattern's procedural, with `size` (metres) standing in for a
/// `frequency` it does not give.
fn pattern_procedural(procedural: Option<&json::Value>, size: Option<&json::Value>) -> Option<tequila_texture::procedural::Procedural> {
    let mut procedural = procedural?.clone();
    // Marble's bands are a sine of its frequency, so one band is 2π over it;
    // every other pattern repeats once per 1 / frequency.
    let per_repeat = if procedural.get("kind").and_then(|k| k.as_str()) == Some("marble") { std::f64::consts::TAU } else { 1.0 };
    if let (Some(size), json::Value::Object(fields)) = (size.and_then(|v| v.as_f64()).filter(|s| s.is_finite() && *s > 0.0), &mut procedural) {
        fields.entry("frequency".into()).or_insert(json::Value::Number(per_repeat / size));
    }
    crate::graph::procedural_from_json(&procedural)
}

/// A pattern laid over what drives `param` already, or over its value. See
/// [`apply_pattern`].
fn layer_pattern(
    graph: &mut MaterialGraph,
    param: Param,
    procedural: tequila_texture::procedural::Procedural,
    seed: u32,
    pattern: &json::Value,
    name: &str,
) -> Result<(), FormatError> {
    use tequila_core::math::vec2;
    use tequila_materials::graph::{Node, NodeKind, Socket, Value};
    use tequila_materials::material_type::ParamUi;

    let invalid = |what: &str| FormatError::Invalid(format!("{name}: a pattern {what}"));
    let output = graph.output();
    let colour = param.ui() == ParamUi::Color;
    let bump = param == Param::Bump;

    // Where the combined value goes, and what it combines with: on the bump,
    // the height going into its Bump node; elsewhere, the parameter itself.
    let (target, socket, base) = if bump {
        let depth = pattern.get("depth").and_then(|v| v.as_f32());
        let existing = graph.driver_of(param).filter(|d| matches!(graph.node(*d).map(|n| &n.kind), Some(NodeKind::Bump { .. })));
        let bump_node = match existing {
            Some(node) => node,
            None => {
                let node = graph.add_node(Node::new(NodeKind::Bump { depth: depth.unwrap_or(0.0002) }));
                graph.connect(node, output, Socket::Param(param));
                node
            }
        };
        if let (Some(depth), Some(NodeKind::Bump { depth: d })) = (depth, graph.node_mut(bump_node).map(|n| &mut n.kind)) {
            *d = depth;
        }
        let height = graph
            .links()
            .iter()
            .find(|l| l.to == bump_node && l.to_socket == Socket::Input(0))
            .map(|l| l.from);
        let base = match height {
            Some(node) => node,
            None => graph.add_node(Node::new(NodeKind::Constant(Value::Float(0.0)))),
        };
        (bump_node, Socket::Input(0), base)
    } else {
        let base = match graph.driver_of(param) {
            Some(driver) => driver,
            None => {
                let value = match param.read(graph.material()) {
                    Some(ParamValue::Scalar(v)) => Value::Float(v),
                    Some(ParamValue::Color(c)) => Value::Color(c),
                    _ => return Err(invalid(&format!("layers onto `{}`, which cannot be patterned", param.key()))),
                };
                graph.add_node(Node::new(NodeKind::Constant(value)))
            }
        };
        (output, Socket::Param(param), base)
    };

    // The layer itself: the pattern, read through its ramp or range.
    let source = graph.add_node(Node::new(NodeKind::Procedural { procedural: Box::new(procedural), seed }));
    let mut layer = source;
    match pattern.get("colors") {
        Some(colors) if colour => {
            let gradient = crate::graph::gradient_from_json(colors).ok_or_else(|| invalid("has an unreadable colour ramp"))?;
            let ramp = graph.add_node(Node::new(NodeKind::Gradient { gradient: Box::new(gradient) }));
            graph.connect(source, ramp, Socket::Input(0));
            layer = ramp;
        }
        Some(_) => return Err(invalid(&format!("gives colours to `{}`, which is not a colour", param.key()))),
        None if colour => {
            let ramp = graph.add_node(Node::new(NodeKind::Gradient { gradient: Box::default() }));
            graph.connect(source, ramp, Socket::Input(0));
            layer = ramp;
        }
        None => {}
    }
    if let Some(range) = pattern.get("range").map(|v| v.as_array()) {
        let (Some(low), Some(high)) = (range.first().and_then(|v| v.as_f32()), range.get(1).and_then(|v| v.as_f32())) else {
            return Err(invalid("has a range that is not two numbers"));
        };
        let remap = graph.add_node(Node::new(NodeKind::Remap { from: vec2(0.0, 1.0), to: vec2(low, high) }));
        graph.connect(layer, remap, Socket::Input(0));
        layer = remap;
    }

    // Where it shows: a mask, cut from this pattern or another, times the
    // amount; or the amount alone.
    let amount = pattern.get("amount").and_then(|v| v.as_f32()).unwrap_or(1.0).clamp(0.0, 1.0);
    let factor = match pattern.get("mask") {
        Some(mask) => {
            let from = match mask.get("procedural") {
                Some(other) => {
                    let other = pattern_procedural(Some(other), mask.get("size")).ok_or_else(|| invalid("has a mask with no procedural this version knows"))?;
                    let other_seed = mask.get("seed").and_then(|v| v.as_f32()).unwrap_or(seed as f32 + 1.0) as u32;
                    graph.add_node(Node::new(NodeKind::Procedural { procedural: Box::new(other), seed: other_seed }))
                }
                None => source,
            };
            let threshold = mask.get("threshold").and_then(|v| v.as_f32()).unwrap_or(0.5);
            let softness = mask.get("softness").and_then(|v| v.as_f32()).unwrap_or(0.05).max(1e-4);
            let cut = graph.add_node(Node::new(NodeKind::Remap { from: vec2(threshold - softness, threshold + softness), to: vec2(0.0, 1.0) }));
            graph.connect(from, cut, Socket::Input(0));
            let clamp = graph.add_node(Node::new(NodeKind::Clamp { low: 0.0, high: 1.0 }));
            graph.connect(cut, clamp, Socket::Input(0));
            let mut factor = clamp;
            if mask.get("invert").and_then(|v| v.as_bool()).unwrap_or(false) {
                let invert = graph.add_node(Node::new(NodeKind::Invert));
                graph.connect(factor, invert, Socket::Input(0));
                factor = invert;
            }
            if amount < 1.0 {
                let scaled = graph.add_node(Node::new(NodeKind::Multiply).with_literal(Socket::Input(1), Value::Float(amount)));
                graph.connect(factor, scaled, Socket::Input(0));
                factor = scaled;
            }
            Some(factor)
        }
        None => None,
    };

    // Combined with what was there.
    let blend = pattern.get("blend").and_then(|v| v.as_str()).unwrap_or("mix");
    let over = match blend {
        "mix" => layer,
        "multiply" | "add" => {
            let kind = if blend == "multiply" { NodeKind::Multiply } else { NodeKind::Add };
            let combined = graph.add_node(Node::new(kind));
            graph.connect(base, combined, Socket::Input(0));
            graph.connect(layer, combined, Socket::Input(1));
            combined
        }
        other => return Err(invalid(&format!("blends by `{other}`; the blends are mix, multiply and add"))),
    };
    let mix = graph.add_node(Node::new(NodeKind::Mix).with_literal(Socket::Input(2), Value::Float(amount)));
    graph.connect(base, mix, Socket::Input(0));
    graph.connect(over, mix, Socket::Input(1));
    if let Some(factor) = factor {
        graph.connect(factor, mix, Socket::Input(2));
    }
    graph.connect(mix, target, socket);
    Ok(())
}

/// Serialize an entry back into the form [`read_entry`] accepts.
///
/// Only what differs from the material type's defaults is written. That is the
/// same principle the reader is built on — a preset is a *recipe*, "a plastic
/// with this colour and this roughness", not a dump of ninety parameters — and
/// it is what keeps a saved preset readable and diffable. It also means a
/// preset tracks improvements to the type's defaults instead of freezing
/// whatever they were on the day it was saved.
pub fn write_entry(entry: &LibraryEntry) -> String {
    write_entry_in(entry, None)
}

/// [`write_entry`], with the paths of pictures inside `directory` written
/// relative to it, so a library can be moved or synced as a folder.
fn write_entry_in(entry: &LibraryEntry, directory: Option<&Path>) -> String {
    use crate::json_write::Object;

    let mut object = Object::new();
    object.set("name", entry.name.clone());
    object.set("type", entry.material_type.id());
    object.set("category", entry.category.clone());
    if !entry.tags.is_empty() {
        object.set(
            "tags",
            entry
                .tags
                .iter()
                .map(|t| crate::json::Value::String(t.clone()))
                .collect::<Vec<crate::json::Value>>(),
        );
    }

    let defaults = entry.material_type.defaults();
    for param in Param::ALL {
        let (Some(mine), Some(theirs)) = (param.read(&entry.material), param.read(&defaults))
        else {
            continue;
        };
        if mine == theirs {
            continue;
        }
        match mine {
            ParamValue::Scalar(v) => {
                object.set(param.key(), v);
            }
            ParamValue::Color(c) => {
                object.set(param.key(), crate::color(c));
            }
            ParamValue::Flag(f) => {
                object.set(param.key(), f);
            }
            // Written by name, the way the reader takes it. A complex index of
            // refraction round-tripped as numbers would stop being the
            // measured value the moment the table was corrected.
            ParamValue::Metal(ior) => {
                if let Some((label, _)) = metals::ALL.iter().find(|(_, m)| *m == ior) {
                    object.set("metal_preset", *label);
                }
            }
        }
    }

    // Patterns, as the graph they built: a saved preset records what it is,
    // and the recipe form is for writing by hand.
    if let Some(graph) = crate::graph::graph_to_json(&entry.graph) {
        object.set("graph", graph);
    }
    if !entry.textures.is_empty() {
        let textures: Vec<json::Value> = entry
            .textures
            .iter()
            .map(|(id, source)| {
                let path = directory
                    .and_then(|d| source.path.strip_prefix(d).ok())
                    .map(|relative| {
                        relative
                            .components()
                            .map(|c| c.as_os_str().to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                            .join("/")
                    })
                    .unwrap_or_else(|| source.path.to_string_lossy().into_owned());
                let mut texture = Object::new();
                texture
                    .set("id", id.0 as f64)
                    .set("path", path)
                    .set("role", crate::journal::role_name(source.role));
                if let Some(space) = source.color_space {
                    texture.set("color_space", crate::journal::color_space_name(space));
                }
                texture.build()
            })
            .collect();
        object.set("textures", textures);
    }

    let mut text = crate::json_write::to_string(&object.build());
    text.push('\n');
    text
}

/// A filename for an entry, derived from its name.
///
/// Lower case, words joined by underscores, and nothing outside
/// `[a-z0-9_]` — so a material called `Anodised 6061 / Blue` cannot write
/// itself into a directory it was not given, or produce a name the filesystem
/// refuses.
pub fn file_stem(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_was_gap = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_was_gap = false;
        } else if !last_was_gap {
            out.push('_');
            last_was_gap = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "material".to_string()
    } else {
        out
    }
}

/// Write an entry into `directory`, returning the file it went to.
///
/// The name is made unique against what is already there rather than
/// overwriting, because two materials a person happens to call the same thing
/// are still two materials, and losing one silently is the worse failure.
pub fn save_entry(
    directory: &Path,
    entry: &LibraryEntry,
) -> Result<std::path::PathBuf, FormatError> {
    std::fs::create_dir_all(directory)
        .map_err(|e| FormatError::Invalid(format!("{}: {e}", directory.display())))?;

    let stem = file_stem(&entry.name);
    let mut path = directory.join(format!("{stem}.json"));
    let mut suffix = 2;
    while path.exists() || pictures_of(&path).exists() {
        path = directory.join(format!("{stem}_{suffix}.json"));
        suffix += 1;
    }

    // The pictures, copied beside the entry: a preset that pointed into a
    // project's folder, or a Downloads folder, would break the day either
    // was tidied.
    let mut saved = entry.clone();
    if !entry.textures.is_empty() {
        let pictures = pictures_of(&path);
        std::fs::create_dir_all(&pictures)
            .map_err(|e| FormatError::Invalid(format!("{}: {e}", pictures.display())))?;
        let mut used = std::collections::BTreeSet::new();
        for (id, source) in &mut saved.textures {
            let name = source
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("texture_{}", id.0));
            // Two maps with one file name, from two folders, stay two files.
            let name = if used.insert(name.to_lowercase()) {
                name
            } else {
                format!("{}_{name}", id.0)
            };
            let copy = pictures.join(&name);
            std::fs::copy(&source.path, &copy).map_err(|e| {
                let _ = std::fs::remove_dir_all(&pictures);
                FormatError::Invalid(format!("{}: {e}", source.path.display()))
            })?;
            source.path = copy;
        }
    }
    std::fs::write(&path, write_entry_in(&saved, Some(directory)))
        .map_err(|e| FormatError::Invalid(format!("{}: {e}", path.display())))?;
    Ok(path)
}

/// Where an entry's pictures are kept: a folder named after its file, in a
/// `textures` folder beside it.
fn pictures_of(entry: &Path) -> PathBuf {
    let stem = entry.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    entry.parent().unwrap_or(Path::new(".")).join("textures").join(stem)
}

/// Rewrite an entry in place, at the path it already has.
pub fn update_entry(entry: &LibraryEntry) -> Result<(), FormatError> {
    if !entry.is_editable() {
        return Err(FormatError::Invalid(format!(
            "`{}` is part of the shipped library and cannot be changed",
            entry.name
        )));
    }
    let directory = Path::new(&entry.path).parent();
    std::fs::write(&entry.path, write_entry_in(entry, directory))
        .map_err(|e| FormatError::Invalid(format!("{}: {e}", entry.path)))
}

/// Remove a user entry's file.
pub fn delete_entry(entry: &LibraryEntry) -> Result<(), FormatError> {
    if !entry.is_editable() {
        return Err(FormatError::Invalid(format!(
            "`{}` is part of the shipped library and cannot be removed",
            entry.name
        )));
    }
    std::fs::remove_file(&entry.path)
        .map_err(|e| FormatError::Invalid(format!("{}: {e}", entry.path)))?;
    // Its own copies of its pictures go with it; nothing else refers to
    // them. A project that used the entry keeps working only while they
    // exist, which is the same bargain as deleting any file a project names.
    let pictures = pictures_of(Path::new(&entry.path));
    if pictures.is_dir() {
        let _ = std::fs::remove_dir_all(&pictures);
    }
    Ok(())
}

/// Where the user's own materials live: beside the rest of the person's
/// per-user state rather than in the installation, which is replaceable
/// and often read-only - a library that vanished when the application was
/// updated would be the worst failure for the one part of it a person built
/// themselves. The same folder for the application, a headless session and
/// a script.
pub fn user_directory() -> PathBuf {
    settings_directory().join("library")
}

/// Tequila's folder among the person's settings, which holds their
/// library, plugins, recent projects and appearance: `%APPDATA%\Tequila`
/// on Windows, `~/Library/Application Support/Tequila` on a Mac, and
/// `$XDG_CONFIG_HOME/Tequila` (or `~/.config/Tequila`) elsewhere.
pub fn settings_directory() -> PathBuf {
    settings_directory_for(cfg!(target_os = "macos"), |name| std::env::var_os(name))
}

fn settings_directory_for(mac: bool, var: impl Fn(&str) -> Option<std::ffi::OsString>) -> PathBuf {
    let home = || var("HOME").map(PathBuf::from);
    let base = if mac {
        home().map(|h| h.join("Library").join("Application Support"))
    } else {
        var("APPDATA")
            .map(PathBuf::from)
            .or_else(|| var("XDG_CONFIG_HOME").map(PathBuf::from))
            .or_else(|| home().map(|h| h.join(".config")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("Tequila")
}

/// Tequila's folder of caches, things it can always make again: in
/// `%LOCALAPPDATA%` on Windows, `~/Library/Caches` on a Mac, and
/// `$XDG_CACHE_HOME` (or `~/.cache`) elsewhere.
pub fn cache_directory() -> PathBuf {
    cache_directory_for(cfg!(target_os = "macos"), |name| std::env::var_os(name))
}

fn cache_directory_for(mac: bool, var: impl Fn(&str) -> Option<std::ffi::OsString>) -> PathBuf {
    let home = || var("HOME").map(PathBuf::from);
    let base = if mac {
        home().map(|h| h.join("Library").join("Caches"))
    } else {
        var("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| var("XDG_CACHE_HOME").map(PathBuf::from))
            .or_else(|| home().map(|h| h.join(".cache")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("Tequila")
}

/// The application's name before 2026-10-17, which its folders still carry
/// on a computer that had it.
const OLDER_NAME: &str = "Freeshot";

/// Bring the person's things across from Freeshot, as the application was
/// called before: their library, plugins, settings, recent projects and
/// appearance, the thumbnails, the network-rendering list and the render
/// history and queue. Once - only where Tequila has nothing of its own yet -
/// and copied, not moved, so a Freeshot still installed keeps what it had.
/// Bundled plugins were `freeshot.<name>` and are `tequila.<name>` now, so
/// the ones that were on stay on. Says what came across.
pub fn adopt_older_name() -> Vec<String> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    let settings = settings_directory();
    let cache = cache_directory();
    let sibling = |folder: &Path| folder.parent().map(|p| p.join(OLDER_NAME));
    adopt_from(&Adoption {
        settings: sibling(&settings).map(|old| (old, settings.clone())),
        cache: sibling(&cache).map(|old| (old, cache.clone())),
        home: home.map(|h| (h.join(".freeshot"), h.join(".tequila"))),
    })
}

/// Where an older name's things are, and where they go: (from, to).
struct Adoption {
    settings: Option<(PathBuf, PathBuf)>,
    cache: Option<(PathBuf, PathBuf)>,
    home: Option<(PathBuf, PathBuf)>,
}

fn adopt_from(adoption: &Adoption) -> Vec<String> {
    let mut brought = Vec::new();
    if let Some((old, new)) = &adoption.settings {
        if old.is_dir() && !new.exists() && copy_tree(old, new).is_ok() {
            rename_bundled_plugins(new);
            brought.push(format!("your materials, plugins and settings, from {}", old.display()));
        }
    }
    if let Some((old, new)) = &adoption.cache {
        let (old, new) = (old.join("thumbnails"), new.join("thumbnails"));
        if old.is_dir() && !new.exists() && copy_tree(&old, &new).is_ok() {
            brought.push("the material thumbnails".into());
        }
    }
    if let Some((old, new)) = &adoption.home {
        // Not the crash journal: a session's files name the folder they are
        // in. The render history and queue that live beside it, yes.
        for file in [Path::new("farm.txt"), Path::new("journal/gallery.json"), Path::new("journal/queue.json")] {
            let (from, to) = (old.join(file), new.join(file));
            if from.is_file() && !to.exists() {
                if let Some(parent) = to.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if std::fs::copy(&from, &to).is_ok() {
                    brought.push(format!("{}", file.display()));
                }
            }
        }
    }
    brought
}

/// The bundled plugins' new ids, in the plugins that are on and in their
/// own folders of settings.
fn rename_bundled_plugins(settings: &Path) {
    let old = format!("\"{}.", OLDER_NAME.to_lowercase());
    let list = settings.join("plugins.json");
    if let Ok(text) = std::fs::read_to_string(&list) {
        let _ = std::fs::write(&list, text.replace(&old, "\"tequila."));
    }
    if let Ok(folders) = std::fs::read_dir(settings.join("plugin-data")) {
        for folder in folders.flatten() {
            let name = folder.file_name().to_string_lossy().into_owned();
            if let Some(rest) = name.strip_prefix(&old[1..]) {
                let _ = std::fs::rename(folder.path(), folder.path().with_file_name(format!("tequila.{rest}")));
            }
        }
    }
}

/// Copy a folder and everything in it.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Where the shipped library lives: searched upwards from the executable,
/// since the depth differs by how the binary was produced - beside it once
/// installed, two levels up from `target/release`, three from a test
/// binary in `deps`, in `Resources` in a Mac bundle - and then in the
/// checkout this was built from.
pub fn standard_directory() -> PathBuf {
    const RELATIVE: &str = "assets/library/standard";
    if let Ok(exe) = std::env::current_exe() {
        let mut candidate = exe;
        for _ in 0..6 {
            if !candidate.pop() {
                break;
            }
            for path in [candidate.join(RELATIVE), candidate.join("Resources").join(RELATIVE)] {
                if path.is_dir() {
                    return path;
                }
            }
        }
    }
    let built_from = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(RELATIVE);
    if built_from.is_dir() {
        return built_from;
    }
    PathBuf::from(RELATIVE)
}

/// Both libraries, the shipped one first: what a browser, a script or a
/// headless session can choose from. Problems with single presets are
/// returned rather than failing the lot.
pub fn read_libraries(standard: &Path, user: &Path) -> (Vec<LibraryEntry>, Vec<String>) {
    let (mut entries, mut problems) = read_directory_from(standard, Source::Standard);
    let (mine, mut mine_problems) = read_directory_from(user, Source::User);
    entries.extend(mine);
    problems.append(&mut mine_problems);
    (entries, problems)
}

/// A copy of `entry` in the user's library at `directory`, under `name`:
/// how a shipped preset becomes a starting point somebody can change, and
/// how one of their own is forked.
pub fn save_copy(entry: &LibraryEntry, directory: &Path, name: &str) -> Result<PathBuf, FormatError> {
    let mut copy = entry.clone();
    copy.name = name.to_string();
    copy.source = Source::User;
    copy.path = String::new();
    save_entry(directory, &copy)
}

/// A user entry renamed, filed under another category, or tagged
/// differently, rewritten where it is. Its file keeps its name: the file is
/// where it lives, not what it is called.
pub fn edit_details(entry: &LibraryEntry, name: &str, category: &str, tags: &[String]) -> Result<LibraryEntry, FormatError> {
    let mut edited = entry.clone();
    edited.name = name.trim().to_string();
    edited.category = category.trim().to_string();
    edited.tags = tags.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    if edited.name.is_empty() || edited.category.is_empty() {
        return Err(FormatError::Invalid("a material needs a name and a category".into()));
    }
    update_entry(&edited)?;
    Ok(edited)
}

/// A user entry's material replaced by `replacement`'s - a material
/// tuned on a part, saved back over the preset it started from - keeping
/// the entry's name, category and tags. Saved before the old one is
/// removed, so a failure leaves the old one where it was; the file may be
/// renamed in passing, never lost.
pub fn replace_material(entry: &LibraryEntry, directory: &Path, replacement: &LibraryEntry) -> Result<PathBuf, FormatError> {
    if !entry.is_editable() {
        return Err(FormatError::Invalid(format!(
            "`{}` is part of the shipped library and cannot be changed",
            entry.name
        )));
    }
    let mut next = replacement.clone();
    next.name = entry.name.clone();
    next.category = entry.category.clone();
    next.tags = entry.tags.clone();
    next.source = Source::User;
    next.path = String::new();
    let saved = save_entry(directory, &next)?;
    // The old file only. Its pictures stay: applying the preset wrote their
    // paths into the project it went on, and into every project saved since,
    // and an update is not the person deleting anything - removing them
    // left those projects pointing at files that were gone.
    std::fs::remove_file(&entry.path).map_err(|e| FormatError::Invalid(format!("{}: {e}", entry.path)))?;
    Ok(saved)
}

/// The file extension of a material package.
pub const PACKAGE_EXTENSION: &str = "fsmat";

/// A material package: one entry and its pictures in one file, for handing
/// a material to somebody else or keeping it outside any library. A ZIP
/// holding `material.json` - the preset as [`write_entry`] writes it - and
/// its pictures under `textures/`, named there as the preset names them.
pub fn package(entry: &LibraryEntry) -> Result<Vec<u8>, FormatError> {
    let mut zip = crate::zip::ZipWriter::new();
    let mut packed = entry.clone();
    packed.path = String::new();
    let mut used = std::collections::BTreeSet::new();
    for (id, source) in &mut packed.textures {
        let bytes = std::fs::read(&source.path).map_err(|e| FormatError::Invalid(format!("{}: {e}", source.path.display())))?;
        let name = source
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("texture_{}", id.0));
        let name = if used.insert(name.to_lowercase()) { name } else { format!("{}_{name}", id.0) };
        zip.add(format!("textures/{name}"), bytes);
        source.path = PathBuf::from(format!("textures/{name}"));
    }
    zip.add("material.json", write_entry(&packed).into_bytes());
    Ok(zip.finish())
}

/// [`package`], written to `to`.
pub fn export_package(entry: &LibraryEntry, to: &Path) -> Result<(), FormatError> {
    let bytes = package(entry)?;
    std::fs::write(to, bytes).map_err(|e| FormatError::Invalid(format!("{}: {e}", to.display())))
}

/// Whether `path` is a material this can bring into a library: a package
/// (`.fsmat`, or any ZIP with a `material.json` in it) or a preset's `.json`.
/// A texture pack's ZIP is not one - it holds pictures and no material.
pub fn is_material_file(path: &Path) -> bool {
    let extension = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match extension.as_str() {
        "fsmat" => true,
        "zip" => std::fs::read(path)
            .ok()
            .and_then(|bytes| crate::zip::ZipArchive::read(&bytes).ok())
            .is_some_and(|archive| archive.contains("material.json")),
        "json" => std::fs::read_to_string(path).ok().is_some_and(|text| {
            let hint = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            read_entry(&text, &hint).is_ok()
        }),
        _ => false,
    }
}

/// Bring the material in `path` - a package or a preset - into the library
/// at `directory`, its pictures copied in with it, after `adjust` has had
/// its say (a new name, another category). Returns the file it went to.
pub fn import_material_file(path: &Path, directory: &Path, adjust: impl FnOnce(&mut LibraryEntry)) -> Result<PathBuf, FormatError> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let scratch = std::env::temp_dir().join(format!("tequila-material-import-{}-{stamp}", std::process::id()));
    let result = read_material_file(path, &scratch).and_then(|mut entry| {
        adjust(&mut entry);
        entry.source = Source::User;
        entry.path = String::new();
        save_entry(directory, &entry)
    });
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// The entry in a material file, its pictures where they can be copied
/// from: beside a preset, as its relative paths say; out of a package, into
/// `scratch`, by their bare file names - nothing in a package is written
/// anywhere its own paths point.
fn read_material_file(path: &Path, scratch: &Path) -> Result<LibraryEntry, FormatError> {
    let hint = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "material".into());
    let unreadable = |e: std::io::Error| FormatError::Invalid(format!("{}: {e}", path.display()));
    let extension = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if extension == "json" {
        let text = std::fs::read_to_string(path).map_err(unreadable)?;
        let mut entry = read_entry(&text, &hint)?;
        let beside = path.parent().unwrap_or(Path::new("."));
        for (_, texture) in &mut entry.textures {
            if texture.path.is_relative() {
                texture.path = beside.join(&texture.path);
            }
            if !texture.path.is_file() {
                return Err(FormatError::Invalid(format!("{}: its picture {} is missing", path.display(), texture.path.display())));
            }
        }
        return Ok(entry);
    }
    let bytes = std::fs::read(path).map_err(unreadable)?;
    let archive = crate::zip::ZipArchive::read(&bytes).map_err(|e| FormatError::Invalid(format!("{}: {e:?}", path.display())))?;
    let text = archive
        .get("material.json")
        .ok_or_else(|| FormatError::Invalid(format!("{} holds no material.json", path.display())))?;
    let text = String::from_utf8(text.to_vec()).map_err(|_| FormatError::Invalid(format!("{}: material.json is not text", path.display())))?;
    let mut entry = read_entry(&text, &hint)?;
    if !entry.textures.is_empty() {
        std::fs::create_dir_all(scratch).map_err(|e| FormatError::Invalid(format!("{}: {e}", scratch.display())))?;
    }
    let mut used = std::collections::BTreeSet::new();
    for (id, texture) in &mut entry.textures {
        let inside = texture.path.to_string_lossy().replace('\\', "/");
        if texture.path.is_absolute() || inside.starts_with('/') || inside.split('/').any(|part| part == ".." || part.contains(':')) {
            return Err(FormatError::Invalid(format!("{}: a picture path that leaves the package: {inside}", path.display())));
        }
        let picture = archive
            .get(&inside)
            .ok_or_else(|| FormatError::Invalid(format!("{}: its picture {inside} is missing", path.display())))?;
        let name = inside.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("texture").to_string();
        let name = if used.insert(name.to_lowercase()) { name } else { format!("{}_{name}", id.0) };
        let file = scratch.join(&name);
        std::fs::write(&file, picture).map_err(|e| FormatError::Invalid(format!("{}: {e}", file.display())))?;
        texture.path = file;
    }
    Ok(entry)
}

/// What putting `entry` on `nodes` does to `document`, as commands to apply
/// in order: the material added - unless an identical one from the library
/// is already there, which is reused, so a preset dropped on five parts is
/// one material and not five - then assigned. `unlink` always adds a copy
/// of its own, named as one.
pub fn apply_commands(
    entry: &LibraryEntry,
    document: &tequila_scene::SceneDocument,
    nodes: &[tequila_scene::tree::NodeId],
    unlink: bool,
) -> Vec<tequila_scene::Command> {
    use tequila_scene::Command;
    // On the parts: a group named here stands for everything in it.
    let nodes = tequila_scene::apply::parts_of(document, nodes.iter().copied());
    if nodes.is_empty() {
        return Vec::new();
    }
    // The graph as this document would hold it: its pictures renumbered
    // into the document's own ids. The whole graph, not only the surface
    // values: a copy whose patterns were edited is not the library's any
    // more.
    let wanted = entry.for_document(document).0.graph.content_hash();
    let existing = (!unlink)
        .then(|| {
            document
                .materials
                .iter()
                .find(|(_, d)| d.name == entry.name && d.material() == &entry.material && d.graph.content_hash() == wanted)
                .map(|(key, _)| key)
        })
        .flatten();
    let mut commands = Vec::new();
    let key = match existing {
        Some(key) => key,
        None => {
            let (mut definition, textures) = entry.for_document(document);
            if unlink {
                // A distinct name, so the library badge does not claim the
                // two are the same instance.
                definition.name = format!("{} (copy)", entry.name);
            }
            let key = tequila_scene::tree::MaterialKey(document.materials.next_key());
            commands.push(Command::Import {
                payload: Box::new(tequila_scene::command::ImportPayload {
                    meshes: Vec::new(),
                    textures,
                    materials: vec![(key, definition)],
                    nodes: Vec::new(),
                    roots: Vec::new(),
                }),
            });
            key
        }
    };
    commands.push(Command::AssignMaterial { nodes, material: key });
    commands
}

/// Read every `.json` preset in a directory, sorted by name.
///
/// A file that will not parse is reported and skipped rather than failing the
/// whole library: one bad preset should not leave the browser empty, and the
/// person who can fix it needs to know which one it was.
pub fn read_directory(directory: &Path) -> (Vec<LibraryEntry>, Vec<String>) {
    read_directory_from(directory, Source::Standard)
}

/// The same, recording where the entries came from.
pub fn read_directory_from(
    directory: &Path,
    source: Source,
) -> (Vec<LibraryEntry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut problems = Vec::new();

    let Ok(listing) = std::fs::read_dir(directory) else {
        // A user library that does not exist yet is the normal first-run
        // state, not a fault to report.
        if source == Source::User && !directory.exists() {
            return (entries, problems);
        }
        return (entries, vec![format!("{} cannot be read", directory.display())]);
    };

    let mut paths: Vec<_> = listing
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    paths.sort();

    for path in paths {
        let hint = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("preset")
            .to_string();
        match std::fs::read_to_string(&path) {
            Ok(text) => match read_entry(&text, &hint) {
                Ok(mut entry) => {
                    entry.source = source;
                    for (_, texture) in &mut entry.textures {
                        if texture.path.is_relative() {
                            texture.path = directory.join(&texture.path);
                        }
                    }
                    entry.path = path.display().to_string();
                    entries.push(entry);
                }
                Err(e) => problems.push(format!("{}: {e}", path.display())),
            },
            Err(e) => problems.push(format!("{}: {e}", path.display())),
        }
    }

    entries.sort_by(|a, b| (&a.category, &a.name).cmp(&(&b.category, &b.name)));
    (entries, problems)
}

/// The categories present, in a stable order for the browser's sidebar.
pub fn categories(entries: &[LibraryEntry]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for entry in entries {
        if !seen.contains(&entry.category) {
            seen.push(entry.category.clone());
        }
    }
    seen.sort();
    seen
}

/// Guard against a preset naming a parameter its type never shows.
///
/// Not an error — a Velvet that sets `coat_weight` is unusual but meaningful —
/// but worth surfacing, because it is more often a typo than an intention.
pub fn unexposed_parameters(entry: &LibraryEntry, text: &str) -> Vec<&'static str> {
    let Ok(value) = json::parse(text.as_bytes()) else {
        return Vec::new();
    };
    let exposed = entry.material_type.exposed();
    Param::ALL
        .iter()
        .filter(|p| value.get(p.key()).is_some())
        .filter(|p| !exposed.contains(p) && p.ui() != ParamUi::Slot)
        .map(|p| p.key())
        .collect()
}

#[cfg(test)]
mod writer_tests {
    use super::*;

    fn library_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("the workspace root")
            .join("assets/library/standard")
    }

    fn sample() -> LibraryEntry {
        let mut material = MaterialType::Metal.defaults();
        material.specular_roughness = 0.17;
        material.base_color = tequila_core::spectrum::spectrum(0.91, 0.86, 0.62);
        LibraryEntry {
            name: "Brass Satin Test".to_string(),
            category: "Metal".to_string(),
            tags: vec!["brass".to_string(), "satin".to_string()],
            material_type: MaterialType::Metal,
            graph: MaterialGraph::from_material(material.clone()),
            material,
            textures: Vec::new(),
            source: Source::User,
            path: String::new(),
        }
    }

    /// The one property a writer owes: what it writes, the reader reads back
    /// as the same thing. Without it, saving a material is a slow way to lose
    /// it.
    #[test]
    fn an_entry_survives_being_written_and_read() {
        let entry = sample();
        let text = write_entry(&entry);
        let read = read_entry(&text, "brass_satin_test").expect("it reads back");

        assert_eq!(read.name, entry.name);
        assert_eq!(read.category, entry.category);
        assert_eq!(read.tags, entry.tags);
        assert_eq!(read.material_type, entry.material_type);
        assert_eq!(read.material, entry.material.sanitized());
    }

    /// Every shipped preset survives it too. One entry round-tripping proves
    /// the mechanism; thirty proves it against the parameters people actually
    /// use, including the ones the sample above happens not to touch.
    #[test]
    fn every_shipped_preset_survives_a_round_trip() {
        let (entries, problems) = read_directory(&library_path());
        assert!(problems.is_empty(), "{problems:?}");
        assert!(entries.len() > 10, "only {} presets", entries.len());

        for entry in entries {
            let text = write_entry(&entry);
            let read = read_entry(&text, &entry.name)
                .unwrap_or_else(|e| panic!("`{}` did not read back: {e}", entry.name));
            assert_eq!(read.material, entry.material, "`{}`", entry.name);
            assert_eq!(read.material_type, entry.material_type, "`{}`", entry.name);
            assert_eq!(read.category, entry.category, "`{}`", entry.name);
        }
    }

    /// A preset's patterns build the graph the Material tab would: a pattern
    /// on a colour through its ramp, on a number through its range, and on
    /// the bump through a bump of the depth asked for. Saved again it is the
    /// same graph, and a pattern that names no parameter is refused.
    #[test]
    fn a_presets_patterns_become_its_graph_and_survive_saving() {
        use tequila_materials::graph::NodeKind;
        let text = r#"{
            "name": "Oak Test", "type": "plastic", "category": "Wood",
            "patterns": [
                {"param": "base_color", "procedural": {"kind": "wood_advanced", "frequency": 60},
                 "colors": [{"at": 0, "color": [0.5, 0.3, 0.13]}, {"at": 1, "color": [0.2, 0.1, 0.04]}]},
                {"param": "specular_roughness", "procedural": {"kind": "noise", "frequency": 200}, "range": [0.4, 0.6]},
                {"param": "bump", "procedural": {"kind": "wood_advanced", "frequency": 60}, "depth": 0.0001}
            ]
        }"#;
        let entry = read_entry(text, "oak_test").expect("reads");
        let kinds: Vec<&NodeKind> = entry.graph.nodes().map(|(_, n)| &n.kind).collect();
        assert!(kinds.iter().any(|k| matches!(k, NodeKind::Gradient { .. })), "no colour ramp");
        assert!(kinds.iter().any(|k| matches!(k, NodeKind::Remap { to, .. } if (to.x - 0.4).abs() < 1e-6)), "no range");
        assert!(kinds.iter().any(|k| matches!(k, NodeKind::Bump { depth } if (*depth - 0.0001).abs() < 1e-9)), "no bump");
        entry.graph.compile().expect("compiles");
        assert_eq!(entry.definition().graph.content_hash(), entry.graph.content_hash());

        let again = read_entry(&write_entry(&entry), "oak_test").expect("reads back");
        assert_eq!(again.graph.content_hash(), entry.graph.content_hash(), "the patterns did not survive saving");

        let wrong = r#"{"name": "X", "type": "plastic", "patterns": [{"param": "shininess", "procedural": {"kind": "noise"}}]}"#;
        assert!(read_entry(wrong, "x").is_err(), "a pattern on no parameter was accepted");
    }

    /// Layers: speckles cut from their own pattern over a mottled base, a
    /// second bump over the first, and a size standing in for a frequency.
    /// Each compiles, and what was there stays under what is laid over it.
    #[test]
    fn patterns_layer_over_each_other() {
        use tequila_materials::graph::NodeKind;
        use tequila_texture::procedural::Procedural;
        let text = r#"{
            "name": "Terrazzo Test", "type": "plastic",
            "patterns": [
                {"param": "base_color", "procedural": {"kind": "noise"}, "size": 0.05,
                 "colors": [{"at": 0, "color": [0.6, 0.6, 0.58]}, {"at": 1, "color": [0.75, 0.74, 0.7]}]},
                {"param": "base_color", "procedural": {"kind": "spots", "coverage": 0.3}, "size": 0.008, "seed": 4,
                 "colors": [{"at": 0, "color": [0.1, 0.1, 0.1]}, {"at": 1, "color": [0.6, 0.2, 0.1]}],
                 "mask": {"threshold": 0.5, "softness": 0.02}},
                {"param": "bump", "procedural": {"kind": "noise", "frequency": 300}, "depth": 0.0001},
                {"param": "bump", "procedural": {"kind": "scratches"}, "blend": "add", "amount": 0.5},
                {"param": "specular_roughness", "procedural": {"kind": "noise"}, "size": 0.02, "range": [0.3, 0.5], "amount": 0.5}
            ]
        }"#;
        let entry = read_entry(text, "terrazzo_test").expect("reads");
        entry.graph.compile().expect("compiles");
        let kinds: Vec<&NodeKind> = entry.graph.nodes().map(|(_, n)| &n.kind).collect();
        assert_eq!(kinds.iter().filter(|k| matches!(k, NodeKind::Mix)).count(), 3, "a layer is a mix over what was there");
        assert_eq!(kinds.iter().filter(|k| matches!(k, NodeKind::Bump { .. })).count(), 1, "the bump layers share one depth");
        assert!(kinds.iter().any(|k| matches!(k, NodeKind::Clamp { .. })), "no mask");
        let frequency = kinds.iter().find_map(|k| match k {
            NodeKind::Procedural { procedural, .. } => match **procedural {
                Procedural::Spots { frequency, .. } => Some(frequency),
                _ => None,
            },
            _ => None,
        });
        assert_eq!(frequency, Some(125.0), "a size of 8 mm is 125 repeats a metre");
        let again = read_entry(&write_entry(&entry), "terrazzo_test").expect("reads back");
        assert_eq!(again.graph.content_hash(), entry.graph.content_hash(), "the layers did not survive saving");

        let wrong = r#"{"name": "X", "type": "plastic", "patterns": [{"param": "base_color", "procedural": {"kind": "noise"}, "blend": "screen"}]}"#;
        assert!(read_entry(wrong, "x").is_err(), "an unknown blend was accepted");
    }

    /// Saving a material to the library keeps its patterns and leaves its
    /// pictures behind: a texture names a file of the project it came from,
    /// and a preset pointing at it would be broken everywhere else.
    #[test]
    fn a_saved_preset_keeps_patterns_and_leaves_pictures() {
        use tequila_materials::graph::{Node, NodeKind, Socket, TextureId, Value};
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let picture = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(3),
            fallback: Value::Float(0.5),
        }));
        let output = graph.output();
        graph.connect(picture, output, Socket::Param(Param::SpecularRoughness));
        let entry = LibraryEntry::from_graph("P".into(), "Plastic".into(), MaterialType::Plastic, &graph, Source::User);
        assert_eq!(entry.graph.len(), 1, "a picture went into the library");
    }

    /// Only what differs from the type's defaults is written. A preset is a
    /// recipe, not a dump: it stays readable, it stays diffable, and it picks
    /// up improvements to the type's defaults instead of freezing whatever
    /// they were the day it was saved.
    #[test]
    fn only_the_differences_are_written() {
        let plain = LibraryEntry {
            name: "Plain".to_string(),
            category: "Plastic".to_string(),
            tags: Vec::new(),
            material_type: MaterialType::Plastic,
            material: MaterialType::Plastic.defaults(),
            graph: MaterialGraph::from_material(MaterialType::Plastic.defaults()),
            textures: Vec::new(),
            source: Source::User,
            path: String::new(),
        };
        let text = write_entry(&plain);
        // Name, type and category, and nothing else.
        let value = crate::json::parse(text.as_bytes()).expect("valid json");
        let crate::json::Value::Object(fields) = value else {
            panic!("not an object");
        };
        let keys: Vec<&str> = fields.keys().map(|k| k.as_str()).collect();
        assert_eq!(keys, vec!["category", "name", "type"], "wrote {keys:?}");
    }

    /// A metal is written by the name of its measured entry, never as raw n
    /// and k. Numbers would stop being the measured value the moment the table
    /// was corrected, and the point of shipping measured data is that it is
    /// the measurement.
    #[test]
    fn a_metal_is_written_by_name() {
        let mut entry = sample();
        entry.material.metal_ior = metals::ALL
            .iter()
            .find(|(l, _)| *l == "Copper")
            .expect("copper is in the table")
            .1;
        let text = write_entry(&entry);
        assert!(text.contains("\"metal_preset\""), "{text}");
        assert!(text.contains("Copper"), "{text}");
        let read = read_entry(&text, "x").expect("reads");
        assert_eq!(read.material.metal_ior, entry.material.metal_ior);
    }

    /// A name is turned into a filename that cannot escape the directory it
    /// was given or upset a filesystem.
    #[test]
    fn a_filename_is_derived_safely() {
        assert_eq!(file_stem("Anodised 6061"), "anodised_6061");
        assert_eq!(file_stem("Blue / Green"), "blue_green");
        assert_eq!(file_stem("../../etc/passwd"), "etc_passwd");
        assert_eq!(file_stem("  "), "material");
        assert_eq!(file_stem("Ünïcode ✨"), "n_code");
    }

    /// Saving never overwrites. Two materials somebody happened to give the
    /// same name are still two materials, and losing one silently is worse
    /// than a file called `brass_2.json`.
    #[test]
    fn saving_twice_writes_two_files() {
        let directory = std::env::temp_dir().join(format!(
            "tequila-library-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let entry = sample();
        let first = save_entry(&directory, &entry).expect("first save");
        let second = save_entry(&directory, &entry).expect("second save");
        assert_ne!(first, second);
        assert!(first.exists() && second.exists());

        let (entries, problems) = read_directory_from(&directory, Source::User);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.source == Source::User));
        assert!(entries.iter().all(|e| e.is_editable()));

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A material made from a texture pack keeps its pictures in the library:
    /// the files are copied beside the entry and named relative to it, read
    /// back to where they are, renumbered into a document without adding a
    /// picture it already has, and removed with the entry.
    #[test]
    fn a_saved_preset_keeps_its_pictures_as_copies() {
        use tequila_materials::graph::{Node, NodeKind, Socket, Value};
        use tequila_texture::texture::MapRole;

        let root = std::env::temp_dir().join(format!("tequila-library-pictures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let pack = root.join("Downloads").join("Leather037");
        let library = root.join("library");
        std::fs::create_dir_all(&pack).unwrap();
        let colour_file = pack.join("Leather037_1K-JPG_Color.jpg");
        std::fs::write(&colour_file, b"colour").unwrap();

        let mut graph = MaterialGraph::from_material(MaterialType::Generic.defaults());
        let picture = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(7),
            fallback: Value::Float(0.5),
        }));
        let output = graph.output();
        graph.connect(picture, output, Socket::Param(Param::BaseColor));
        let mut store = TextureStore::new();
        store.insert_with_id(TextureId(7), TextureSource::new(&colour_file, MapRole::Color));

        let entry = LibraryEntry::from_graph_with_pictures(
            "Leather037".into(),
            "Leather".into(),
            MaterialType::Generic,
            &graph,
            &store,
            Source::User,
        );
        assert_eq!(entry.textures.len(), 1, "the picture was left behind");
        assert_eq!(entry.textures[0].0, TextureId(1));
        let reads = |g: &MaterialGraph, id: TextureId| {
            g.nodes().any(|(_, n)| matches!(n.kind, NodeKind::ImageTexture { texture, .. } if texture == id))
        };
        assert!(reads(&entry.graph, TextureId(1)), "the graph was not renumbered");

        let saved = save_entry(&library, &entry).expect("saves");
        let text = std::fs::read_to_string(&saved).unwrap();
        assert!(text.contains("\"textures/leather037/Leather037_1K-JPG_Color.jpg\""), "{text}");
        assert!(!text.contains("Downloads"), "the entry points outside the library: {text}");
        let copy = library.join("textures").join("leather037").join("Leather037_1K-JPG_Color.jpg");
        assert_eq!(std::fs::read(&copy).unwrap(), b"colour");

        // The pack's folder is tidied away; the library's copy carries on.
        std::fs::remove_dir_all(&pack).unwrap();
        let (entries, problems) = read_directory_from(&library, Source::User);
        assert!(problems.is_empty(), "{problems:?}");
        let read = &entries[0];
        assert_eq!(read.textures[0].1.path, copy);
        assert_eq!(read.textures[0].1.role, MapRole::Color);
        assert!(reads(&read.graph, TextureId(1)));

        // Into a document that already has pictures: new ids past its own,
        // and the second time, the picture it now has rather than another.
        let mut document = tequila_scene::SceneDocument::new();
        document.textures.insert_with_id(TextureId(3), TextureSource::new("other.png", MapRole::Color));
        let (definition, added) = read.for_document(&document);
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].0, TextureId(4));
        assert!(reads(&definition.graph, TextureId(4)));
        document.textures.insert_with_id(added[0].0, added[0].1.clone());
        let (again, added_again) = read.for_document(&document);
        assert!(added_again.is_empty(), "the same picture was added twice");
        assert_eq!(again.graph.content_hash(), definition.graph.content_hash());

        delete_entry(read).expect("deletes");
        assert!(!saved.exists());
        assert!(!copy.exists(), "the entry's pictures outlived it");

        // A picture whose file is gone: the surface is saved without it.
        let lost = LibraryEntry::from_graph_with_pictures(
            "Lost".into(),
            "Leather".into(),
            MaterialType::Generic,
            &graph,
            &store,
            Source::User,
        );
        assert!(lost.textures.is_empty());
        assert_eq!(lost.graph.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A material handed to somebody else as one file: packaged with its
    /// pictures, opened into another library with them copied in, the
    /// graph still reading them - and told apart from a texture pack's ZIP,
    /// which has pictures but no material.
    #[test]
    fn a_package_carries_a_material_and_its_pictures() {
        use tequila_materials::graph::{Node, NodeKind, Socket, Value};
        use tequila_texture::texture::MapRole;

        let root = std::env::temp_dir().join(format!("tequila-library-package-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (mine, theirs) = (root.join("mine"), root.join("theirs"));
        std::fs::create_dir_all(&root).unwrap();
        let colour_file = root.join("Grain_Color.png");
        std::fs::write(&colour_file, b"colour").unwrap();
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let picture = graph.add_node(Node::new(NodeKind::ImageTexture { texture: TextureId(2), fallback: Value::Float(0.5) }));
        let output = graph.output();
        graph.connect(picture, output, Socket::Param(Param::BaseColor));
        let mut store = TextureStore::new();
        store.insert_with_id(TextureId(2), TextureSource::new(&colour_file, MapRole::Color));
        let entry = LibraryEntry::from_graph_with_pictures("Grain Blue".into(), "Plastic".into(), MaterialType::Plastic, &graph, &store, Source::User);
        let saved = save_entry(&mine, &entry).unwrap();
        let (read, _) = read_directory_from(&mine, Source::User);

        let file = root.join("grain_blue.fsmat");
        export_package(&read[0], &file).expect("packages");
        assert!(is_material_file(&file));
        let archive = crate::zip::ZipArchive::read(&std::fs::read(&file).unwrap()).unwrap();
        let text = String::from_utf8(archive.get("material.json").unwrap().to_vec()).unwrap();
        assert!(text.contains("\"textures/Grain_Color.png\""), "{text}");
        assert!(!text.contains(&*root.to_string_lossy().replace('\\', "\\\\")), "the package names a path on this machine: {text}");

        let landed = import_material_file(&file, &theirs, |e| e.category = "Imported".into()).expect("imports");
        let (back, problems) = read_directory_from(&theirs, Source::User);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].name, "Grain Blue");
        assert_eq!(back[0].category, "Imported");
        assert_eq!(back[0].path, landed.display().to_string());
        assert_eq!(std::fs::read(&back[0].textures[0].1.path).unwrap(), b"colour");
        assert!(back[0].textures[0].1.path.starts_with(&theirs), "the picture was not copied into the library");
        assert_eq!(back[0].graph.content_hash(), read[0].graph.content_hash());

        // A preset's .json, read with its pictures where it says they are.
        let again = import_material_file(&saved, &theirs, |_| {}).expect("a preset file imports");
        assert_ne!(again, landed, "the second copy overwrote the first");

        // A texture pack's ZIP is not a material.
        let mut pack = crate::zip::ZipWriter::new();
        pack.add("Bricks_Color.png", b"x".to_vec());
        let pack_file = root.join("Bricks.zip");
        std::fs::write(&pack_file, pack.finish()).unwrap();
        assert!(!is_material_file(&pack_file));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A package whose material names a picture outside itself - `..`, an
    /// absolute path - is refused, and nothing is written where it points.
    #[test]
    fn a_package_cannot_reach_outside_itself() {
        let root = std::env::temp_dir().join(format!("tequila-library-escape-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // The control first: the same package, its picture inside itself.
        for path in ["textures/ok.png", "../escaped.png", "C:/escaped.png", "/escaped.png"] {
            let material = format!(
                r#"{{"name":"Bad","type":"plastic","category":"Plastic","graph":{{"nodes":[{{"id":2,"kind":"image_texture","texture":1,"fallback":{{"float":0.5}}}}],"links":[{{"from":2,"to":1,"socket":"param:base_color"}}],"output":1}},"textures":[{{"id":1,"path":"{path}","role":"color"}}]}}"#
            );
            let mut zip = crate::zip::ZipWriter::new();
            zip.add("material.json", material.into_bytes());
            // Present under exactly that name, so only the guard refuses it.
            zip.add(path, b"x".to_vec());
            let file = root.join("bad.fsmat");
            std::fs::write(&file, zip.finish()).unwrap();
            let result = import_material_file(&file, &root.join("library"), |_| {});
            if path.starts_with("textures/") {
                assert!(result.is_ok(), "the control did not import: {result:?}");
            } else {
                assert!(result.is_err(), "{path} was let through");
            }
        }
        assert!(!root.parent().unwrap().join("escaped.png").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A shipped preset becomes one of the user's own to change: copied,
    /// renamed and refiled, and its material replaced by one tuned on a
    /// part, keeping its name. The copy is editable; the original never is.
    #[test]
    fn a_preset_is_copied_edited_and_replaced_in_the_users_library() {
        let root = std::env::temp_dir().join(format!("tequila-library-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (standard, _) = read_directory(&library_path());
        let abs = standard.iter().find(|e| e.name == "ABS White").expect("ABS White ships");
        assert!(!abs.is_editable());
        let copy_path = save_copy(abs, &root, "My White").expect("copies");
        let (mine, _) = read_directory_from(&root, Source::User);
        let copy = &mine[0];
        assert!(copy.is_editable());
        assert_eq!(copy.name, "My White");
        assert_eq!(copy.graph.content_hash(), abs.graph.content_hash());

        let edited = edit_details(copy, "Housing White", "Enclosures", &["matte".into(), " ".into()]).expect("edits");
        let (mine, _) = read_directory_from(&root, Source::User);
        assert_eq!(mine[0].name, "Housing White");
        assert_eq!(mine[0].category, "Enclosures");
        assert_eq!(mine[0].tags, vec!["matte".to_string()]);
        assert_eq!(mine[0].path, copy_path.display().to_string(), "editing details moved the file");
        assert!(edit_details(&edited, " ", "Enclosures", &[]).is_err());

        let mut tuned = mine[0].clone();
        tuned.material.specular_roughness = 0.6;
        tuned.graph = MaterialGraph::from_material(tuned.material.clone());
        tuned.name = "whatever the part called it".into();
        // A picture of the old one's, as a project it was applied to names.
        let pictures = pictures_of(Path::new(&mine[0].path));
        std::fs::create_dir_all(&pictures).unwrap();
        std::fs::write(pictures.join("base.png"), b"in use").unwrap();
        replace_material(&mine[0], &root, &tuned).expect("replaces");
        assert!(pictures.join("base.png").exists(), "a picture projects use was deleted by an update");
        let (mine, _) = read_directory_from(&root, Source::User);
        assert_eq!(mine.len(), 1, "the old one was left behind");
        assert_eq!(mine[0].name, "Housing White");
        assert_eq!(mine[0].category, "Enclosures");
        assert_eq!(mine[0].material.specular_roughness, 0.6);
        assert!(replace_material(abs, &root, &tuned).is_err(), "a shipped preset was replaced");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Applying a library material is one recipe wherever it is asked for -
    /// the browser, a script, a headless session: added and assigned the
    /// first time, the same instance reused the next, a copy of its own
    /// when unlinked.
    #[test]
    fn applying_a_preset_reuses_the_instance_already_there() {
        use tequila_scene::Command;
        let (standard, _) = read_directory(&library_path());
        let entry = standard.iter().find(|e| e.name == "Aluminium Spun").expect("ships");
        let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
        let mut bus = tequila_scene::command::CommandBus::new();
        let part = document.tree.iter().find(|n| n.mesh().is_some()).map(|n| n.id).expect("a part");
        assert!(apply_commands(entry, &document, &[], false).is_empty());
        let first = apply_commands(entry, &document, &[part], false);
        assert_eq!(first.len(), 2);
        assert!(matches!(first[0], Command::Import { .. }));
        for command in first {
            bus.apply(&mut document, command).expect("applies");
        }
        let count = document.materials.iter().count();
        let second = apply_commands(entry, &document, &[part], false);
        assert_eq!(second.len(), 1, "a second instance was added");
        assert!(matches!(second[0], Command::AssignMaterial { .. }));
        let unlinked = apply_commands(entry, &document, &[part], true);
        assert_eq!(unlinked.len(), 2);
        for command in unlinked {
            bus.apply(&mut document, command).expect("applies");
        }
        assert_eq!(document.materials.iter().count(), count + 1);
        assert!(document.materials.iter().any(|(_, d)| d.name == "Aluminium Spun (copy)"));
    }

    /// A group named - a selected group, or one a script names - stands for
    /// the parts in it. Assigned to the group itself, the material was never
    /// drawn: a group's own material is not, so nothing changed on screen.
    #[test]
    fn a_preset_put_on_a_group_goes_on_its_parts() {
        use tequila_scene::Command;
        let (standard, _) = read_directory(&library_path());
        let entry = standard.iter().find(|e| e.name == "Aluminium Spun").expect("ships");
        let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
        let group = document.tree.iter().find(|n| matches!(n.kind, tequila_scene::tree::NodeKind::Group)).map(|n| n.id).expect("a group");
        let parts = tequila_scene::apply::parts_of(&document, [group]);
        assert!(!parts.is_empty());
        let mut bus = tequila_scene::command::CommandBus::new();
        for command in apply_commands(entry, &document, &[group], false) {
            if let Command::AssignMaterial { nodes, .. } = &command {
                assert_eq!(nodes, &parts, "assigned to the group, not its parts");
            }
            bus.apply(&mut document, command).expect("applies");
        }
        for part in parts {
            let material = document.tree.get(part).and_then(|n| n.material).and_then(|k| document.materials.get(k));
            assert_eq!(material.map(|m| m.name.as_str()), Some("Aluminium Spun"));
        }
    }

    /// The shipped library is never written to, and the guard is in the
    /// operation rather than only in the UI that calls it — a read-only rule
    /// enforced by a greyed-out menu item is a rule one refactor away from
    /// being gone.
    #[test]
    fn the_shipped_library_refuses_to_be_edited() {
        let (entries, _) = read_directory(&library_path());
        let shipped = entries.first().expect("a shipped preset");
        assert_eq!(shipped.source, Source::Standard);
        assert!(!shipped.is_editable());
        assert!(update_entry(shipped).is_err());
        assert!(delete_entry(shipped).is_err());
        // And the file is still there.
        assert!(std::path::Path::new(&shipped.path).exists());
    }

    /// A user library that does not exist yet is the normal first-run state,
    /// not a problem to report at the user.
    #[test]
    fn an_absent_user_library_is_not_an_error() {
        let missing = std::env::temp_dir().join("tequila-nonexistent-library-dir");
        let _ = std::fs::remove_dir_all(&missing);
        let (entries, problems) = read_directory_from(&missing, Source::User);
        assert!(entries.is_empty());
        assert!(problems.is_empty(), "{problems:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("the workspace root")
            .join("assets/library/standard")
    }

    /// A computer that had Freeshot keeps everything in Tequila: copied once,
    /// the bundled plugins that were on still on, nothing of Tequila's own
    /// written over, and Freeshot's folders left as they were.
    #[test]
    fn freeshots_things_come_across_once() {
        let root = std::env::temp_dir().join(format!("tequila-adopt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let old = root.join("Roaming").join("Freeshot");
        std::fs::create_dir_all(old.join("library").join("textures").join("polystyrene")).unwrap();
        std::fs::write(old.join("library").join("polystyrene.json"), "{}").unwrap();
        std::fs::write(old.join("library").join("textures").join("polystyrene").join("diff.jpg"), "jpg").unwrap();
        std::fs::write(old.join("plugins.json"), r#"{"enabled": ["freeshot.vibes", "someone.tool"]}"#).unwrap();
        std::fs::create_dir_all(old.join("plugin-data").join("freeshot.vibes")).unwrap();
        std::fs::write(old.join("appearance.txt"), "mode=light\n").unwrap();
        let old_home = root.join("home").join(".freeshot");
        std::fs::create_dir_all(old_home.join("journal")).unwrap();
        std::fs::write(old_home.join("journal").join("gallery.json"), "[1]").unwrap();
        std::fs::write(old_home.join("journal").join("crashed.jsonl"), "{}").unwrap();
        let old_cache = root.join("Local").join("Freeshot");
        std::fs::create_dir_all(old_cache.join("thumbnails")).unwrap();
        std::fs::write(old_cache.join("thumbnails").join("a.png"), "png").unwrap();

        let new = root.join("Roaming").join("Tequila");
        let new_home = root.join("home").join(".tequila");
        let adoption = Adoption {
            settings: Some((old.clone(), new.clone())),
            cache: Some((old_cache.clone(), root.join("Local").join("Tequila"))),
            home: Some((old_home.clone(), new_home.clone())),
        };
        let brought = adopt_from(&adoption);
        assert_eq!(brought.len(), 3, "{brought:?}");
        assert!(new.join("library").join("textures").join("polystyrene").join("diff.jpg").is_file());
        assert_eq!(std::fs::read_to_string(new.join("plugins.json")).unwrap(), r#"{"enabled": ["tequila.vibes", "someone.tool"]}"#);
        assert!(new.join("plugin-data").join("tequila.vibes").is_dir());
        assert!(new_home.join("journal").join("gallery.json").is_file());
        assert!(!new_home.join("journal").join("crashed.jsonl").exists(), "a crash journal names its own folder");
        assert!(root.join("Local").join("Tequila").join("thumbnails").join("a.png").is_file());
        assert!(old.join("plugins.json").is_file() && old.join("plugin-data").join("freeshot.vibes").is_dir(), "Freeshot's own were changed");

        // Once: Tequila's own, made since, are never written over.
        std::fs::write(new.join("appearance.txt"), "mode=dark\n").unwrap();
        assert!(adopt_from(&adoption).is_empty());
        assert_eq!(std::fs::read_to_string(new.join("appearance.txt")).unwrap(), "mode=dark\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_mac_keeps_settings_and_caches_in_its_library_folder() {
        let environment = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| pairs.iter().find(|(n, _)| *n == name).map(|(_, v)| std::ffi::OsString::from(v))
        };
        // Compared by components, so `/` stands for either separator.
        let path = PathBuf::from;
        // A Mac: the Library, whatever else is set.
        let mac = environment(&[("HOME", "/Users/me"), ("XDG_CONFIG_HOME", "/Users/me/.config"), ("XDG_CACHE_HOME", "/Users/me/.cache")]);
        assert_eq!(settings_directory_for(true, mac), path("/Users/me/Library/Application Support/Tequila"));
        assert_eq!(cache_directory_for(true, mac), path("/Users/me/Library/Caches/Tequila"));
        // Windows, as it has always been.
        let windows = environment(&[("APPDATA", "C:/Users/me/AppData/Roaming"), ("LOCALAPPDATA", "C:/Users/me/AppData/Local"), ("HOME", "C:/Users/me")]);
        assert_eq!(settings_directory_for(false, windows), path("C:/Users/me/AppData/Roaming/Tequila"));
        assert_eq!(cache_directory_for(false, windows), path("C:/Users/me/AppData/Local/Tequila"));
        // Linux: XDG, else the dot folders.
        let linux = environment(&[("HOME", "/home/me"), ("XDG_CACHE_HOME", "/tmp/cache")]);
        assert_eq!(settings_directory_for(false, linux), path("/home/me/.config/Tequila"));
        assert_eq!(cache_directory_for(false, linux), path("/tmp/cache/Tequila"));
        // The library and everything kept beside it hang off the one folder.
        assert_eq!(user_directory().parent(), Some(settings_directory().as_path()));
    }

    /// M2 asks for "50 materials minimum, all as JSON presets", and every one
    /// of them has to load.
    #[test]
    fn the_standard_library_loads_and_is_big_enough() {
        let (entries, problems) = read_directory(&library_path());
        assert!(problems.is_empty(), "presets failed to load: {problems:#?}");
        assert!(
            entries.len() >= 50,
            "the standard library has {} presets; M2 asks for at least 50",
            entries.len()
        );
    }

    /// Every preset must produce a material the renderer will accept — a
    /// library entry that shades wrong is worse than one that is missing,
    /// because it looks like the renderer is broken rather than the file.
    #[test]
    fn every_preset_produces_a_sane_material() {
        use tequila_materials::bsdf::Bsdf;
        use tequila_core::math::{vec2, vec3};

        let (entries, _) = read_directory(&library_path());
        let wo = vec3(0.3, 0.0, 0.954).normalize();

        for entry in &entries {
            // Sanitizing must be a no-op: a preset should already be valid.
            assert_eq!(
                entry.material.clone().sanitized(),
                entry.material,
                "`{}` needed sanitizing, so it holds a value outside its range",
                entry.name
            );

            let bsdf = entry.material.bsdf(0.02);
            // Nothing may return a non-finite value or create energy.
            let mut carried = 0.0f32;
            let samples = 256;
            for i in 0..samples {
                let u = vec2(
                    (i as f32 + 0.5) / samples as f32,
                    ((i * 37) % samples) as f32 / samples as f32,
                );
                let uc = ((i * 13) % samples) as f32 / samples as f32;
                if let Some(sample) = bsdf.sample(wo, u, uc) {
                    assert!(
                        sample.f.is_finite() && sample.pdf.is_finite(),
                        "`{}` produced a non-finite sample",
                        entry.name
                    );
                    if sample.pdf > 0.0 {
                        carried += sample.f.luminance()
                            * sample.wi.z.abs()
                            / sample.pdf;
                    }
                }
            }
            let throughput = carried / samples as f32;
            assert!(
                throughput <= 1.05,
                "`{}` returns {throughput:.3} of the light it receives",
                entry.name
            );
        }
    }

    /// Names have to be unique, or the browser shows two rows a person cannot
    /// tell apart.
    #[test]
    fn preset_names_are_unique() {
        let (entries, _) = read_directory(&library_path());
        let mut seen = std::collections::BTreeSet::new();
        for entry in &entries {
            assert!(
                seen.insert(entry.name.clone()),
                "`{}` appears twice in the library",
                entry.name
            );
        }
    }

    /// §0.2 rule 3: no commercial swatch systems, by name or by code.
    ///
    /// A cheap check, and the kind of thing that slips in when someone adds a
    /// preset in a hurry and names it after the swatch they were matching.
    #[test]
    fn no_preset_names_a_commercial_swatch_system() {
        let (entries, _) = read_directory(&library_path());
        const FORBIDDEN: [&str; 7] = [
            "pantone",
            "ral",
            "mold-tech",
            "sorensen",
            "coloro",
            "interpon",
            "axalta",
        ];
        for entry in &entries {
            // Whole words, not substrings: "Canvas Natu*ral*" is not a swatch
            // reference, and a check that says it is gets disabled rather than
            // fixed.
            let text = format!("{} {} {}", entry.name, entry.category, entry.tags.join(" "))
                .to_lowercase();
            let words: Vec<&str> = text
                .split(|c: char| !c.is_alphanumeric() && c != '-')
                .filter(|w| !w.is_empty())
                .collect();
            for forbidden in FORBIDDEN {
                assert!(
                    !words.contains(&forbidden),
                    "`{}` references {forbidden}, which §0.2 rule 3 forbids bundling",
                    entry.name
                );
            }
        }
    }

    /// Searching matches across name, category and tags, and every word of the
    /// query has to land — so a two-word search narrows rather than widens.
    #[test]
    fn searching_narrows_across_name_category_and_tags() {
        let (entries, _) = read_directory(&library_path());
        let find = |q: &str| -> Vec<&str> {
            entries
                .iter()
                .filter(|e| e.matches(q))
                .map(|e| e.name.as_str())
                .collect()
        };

        assert_eq!(find("").len(), entries.len(), "an empty query matches all");

        let metals = find("metal");
        assert!(!metals.is_empty(), "no metals found by category");

        // A tag nobody's name contains.
        let brushed = find("brushed");
        assert!(
            brushed.iter().any(|n| n.contains("Brushed")),
            "tag search missed: {brushed:?}"
        );

        // Two words narrow.
        let both = find("brushed metal");
        assert!(
            both.len() <= brushed.len() && !both.is_empty(),
            "adding a word widened the result: {} -> {}",
            brushed.len(),
            both.len()
        );

        assert!(find("nonexistentmaterial").is_empty());
    }

    /// A preset that sets a parameter its type never shows is usually a typo.
    #[test]
    fn presets_stay_within_the_parameters_their_type_shows() {
        let mut surprises = Vec::new();
        let directory = library_path();
        let (entries, _) = read_directory(&directory);
        for entry in &entries {
            let slug = entry.name.to_lowercase().replace(' ', "_");
            let Ok(text) = std::fs::read_to_string(directory.join(format!("{slug}.json"))) else {
                continue;
            };
            let unexposed = unexposed_parameters(entry, &text);
            if !unexposed.is_empty() {
                surprises.push(format!("{}: {unexposed:?}", entry.name));
            }
        }
        assert!(
            surprises.is_empty(),
            "presets set parameters their type does not show, which is usually a \
             typo rather than an intention:\n{surprises:#?}"
        );
    }

    /// A malformed preset is reported by name and does not take the library
    /// down with it.
    #[test]
    fn a_bad_preset_is_reported_rather_than_fatal() {
        assert!(read_entry("{ not json", "broken").is_err());
        assert!(read_entry(r#"{"name":"X"}"#, "broken").is_err(), "no type");
        match read_entry(r#"{"name":"X","type":"nonsense"}"#, "broken") {
            Err(FormatError::Invalid(m)) => assert!(m.contains("nonsense"), "{m}"),
            other => panic!("expected a clear error, got {other:?}"),
        }
        match read_entry(
            r#"{"name":"X","type":"metal","metal_preset":"Unobtainium"}"#,
            "broken",
        ) {
            Err(FormatError::Invalid(m)) => assert!(m.contains("Unobtainium"), "{m}"),
            other => panic!("expected a clear error, got {other:?}"),
        }
    }

    /// A preset is a recipe: it records only what it changes, and inherits the
    /// rest from its type. That is what makes retuning a type reach the whole
    /// library.
    #[test]
    fn a_preset_inherits_everything_it_does_not_state() {
        let entry = read_entry(
            r#"{"name":"Test","type":"plastic","specular_roughness":0.5}"#,
            "test",
        )
        .expect("reads");
        let defaults = MaterialType::Plastic.defaults();

        assert_eq!(entry.material.specular_roughness, 0.5, "the stated value");
        assert_eq!(
            entry.material.base_color, defaults.base_color,
            "an unstated parameter should come from the type"
        );
        assert_eq!(entry.material.coat_weight, defaults.coat_weight);
    }

    /// Metals are named, because nobody hand-writes a complex index of
    /// refraction.
    #[test]
    fn a_metal_preset_is_named_not_numbered() {
        let entry = read_entry(
            r#"{"name":"Test","type":"metal","metal_preset":"Gold"}"#,
            "test",
        )
        .expect("reads");
        assert_eq!(entry.material.metal_ior, metals::GOLD);
    }

    #[test]
    fn categories_are_stable_and_deduplicated() {
        let (entries, _) = read_directory(&library_path());
        let categories = categories(&entries);
        assert!(categories.len() >= 4, "got {categories:?}");
        let mut sorted = categories.clone();
        sorted.sort();
        assert_eq!(categories, sorted, "categories must come out in order");
        let unique: std::collections::BTreeSet<_> = categories.iter().collect();
        assert_eq!(unique.len(), categories.len());
    }
}
