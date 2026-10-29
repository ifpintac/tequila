//! The `.tequila` container: read, write, schema validation, migration.
//!
//! `docs/03-SCENE-AND-IO.md` §4. The archive layout, the split of materials
//! and geometry into their own members, and the `.fsmesh` binary layout are
//! all specified there; this implements them.
//!
//! # Two commitments that shape everything here
//!
//! **The round trip is byte-identical.** M1's exit criterion is "save, reopen,
//! and get a byte-identical document", which rules out anything that varies
//! between two writes of one scene: no wall-clock timestamps in the archive,
//! sorted JSON keys, shortest-round-trip number formatting, and members
//! written in a fixed order.
//!
//! **A newer file is not destroyed by an older build.** §4.4: a newer *minor*
//! version loads, warns about fields it does not understand, and **preserves
//! them on save**. Those unknown fields ride along in [`Preserved`], which is
//! handed back on load and passed to save. A build that dropped them would
//! quietly delete the newer build's work on the user's next save, which is the
//! worst failure this format can have.

pub mod exr;
pub mod fsmesh;
pub mod hdri;
pub mod output;
pub mod passes;
pub mod gltf;
pub mod livelink;
pub mod psd;
pub mod spin;
pub mod usd;
pub mod web;
pub mod queue;
pub mod examples;
pub mod journal;
pub mod json;
pub mod json_write;
pub mod zip;

use tequila_color::{DisplayTransform, ImageStyle, ToneMode};
use tequila_core::math::{Mat4, Transform, Vec3};
use tequila_core::spectrum::{spectrum, Spectrum};
use tequila_core::units::LengthUnit;
use tequila_materials::fresnel::ComplexIor;
mod graph;
pub mod library;
pub mod studio_preset;

/// A procedural as the project file and `material.set_procedural` write it:
/// its `kind` and that kind's own fields. What a script or an assistant is
/// shown so it can ask for one without guessing the field names.
pub fn procedural_to_json(procedural: &tequila_texture::procedural::Procedural) -> json::Value {
    graph::procedural_to_json(procedural)
}

use tequila_materials::material_type::{MaterialType, Param, ParamValue};

use tequila_render_core::camera::{Aperture, Camera, Projection};
use tequila_render_core::light::Visibility;
use tequila_render_core::settings::{IntegratorConfig, LightingPreset, LightingSettings};
use tequila_scene::command::{EnvParam, EnvValue};
use tequila_scene::document::{
    BackgroundMode, DocumentMeta, EnvironmentSettings, OcioSelection, SceneDocument,
    FORMAT_VERSION,
};
use tequila_scene::lighting::{
    CameraStore, Intensity, LightSettings, LightShape, SavedCamera,
};
use tequila_scene::render::{
    AlphaMode as SceneAlphaMode, BitDepth, OutputFormat, Region, RenderSettings,
};
use tequila_scene::material::MaterialDefinition;
use tequila_scene::tree::{MaterialKey, MeshKey, NodeId, NodeKind, SceneNode};
use tequila_scene::units::SceneUnits;
use json::Value;
use json_write::Object;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

const SCENE_MEMBER: &str = "scene.json";
const MANIFEST_MEMBER: &str = "manifest.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatError {
    Io(String),
    Archive(zip::ZipError),
    Mesh(String),
    /// The file is structurally sound but says something impossible. §4.2:
    /// "Reject-with-explanation beats silently misparsing."
    Invalid(String),
    /// §4.4: "Newer major: refuse with a clear message."
    TooNew {
        file: String,
        supported: String,
    },
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::Io(m) => write!(f, "{m}"),
            FormatError::Archive(e) => write!(f, "{e}"),
            FormatError::Mesh(m) => write!(f, "{m}"),
            FormatError::Invalid(m) => write!(f, "{m}"),
            FormatError::TooNew { file, supported } => write!(
                f,
                "this file was written by a newer version of Tequila (format {file}); \
                 this build reads format {supported}. Update Tequila to open it."
            ),
        }
    }
}

impl std::error::Error for FormatError {}

impl From<zip::ZipError> for FormatError {
    fn from(e: zip::ZipError) -> Self {
        FormatError::Archive(e)
    }
}

/// Fields a newer build wrote that this one does not understand.
///
/// §4.4 requires these to survive a load-and-save cycle: "this is what lets
/// older builds not destroy newer projects". They are carried opaquely — this
/// build makes no attempt to interpret them — and merged back in on write.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preserved {
    /// Top-level keys of `scene.json` this build does not know.
    scene: BTreeMap<String, Value>,
    /// Per-node keys, by node ID.
    nodes: BTreeMap<u64, BTreeMap<String, Value>>,
    /// Archive members this build does not write.
    members: BTreeMap<String, Vec<u8>>,
    /// Keys inside the scene's settings - the environment and its ground,
    /// the render settings, the image style, the lighting - by the path of
    /// the object they were in (`environment.ground`).
    settings: BTreeMap<String, BTreeMap<String, Value>>,
    /// What each material carried that this build does not understand, by
    /// its key.
    materials: BTreeMap<u64, MaterialExtras>,
}

/// A material's part of [`Preserved`].
#[derive(Clone, Debug, Default, PartialEq)]
struct MaterialExtras {
    /// Keys - parameters, mostly - this build does not know.
    unknown: BTreeMap<String, Value>,
    /// A type, or a graph, this build could not read, as the file had it;
    /// written back while the material is still as it was loaded in their
    /// place (`loaded`). Changed, it is this build's material from then on.
    original_type: Option<String>,
    original_graph: Option<Value>,
    loaded: Option<tequila_scene::material::MaterialDefinition>,
}

/// The settings objects whose unknown keys are kept, by their path in
/// `scene.json`.
const SETTINGS_PATHS: [&str; 8] = [
    "units",
    "lighting",
    "lighting.custom",
    "environment",
    "environment.background",
    "environment.ground",
    "image_style",
    "render",
];

/// Keys this build knows but does not always write, by path: left out when
/// empty, or read from older files and written no more. Kept from being
/// taken for a newer build's - which would bring back a value the person
/// cleared, or apply an old file's scale a second time.
fn known_unwritten(path: &str) -> &'static [&'static str] {
    match path {
        "environment" => &["image", "sky", "pins"],
        "units" => &["metres_per_unit", "meters_per_unit"],
        "lighting.custom" => &["cycles"],
        _ => &[],
    }
}

impl Preserved {
    pub fn is_empty(&self) -> bool {
        self.scene.is_empty() && self.nodes.is_empty() && self.members.is_empty() && self.settings.is_empty() && self.materials.is_empty()
    }

    /// What was carried through, for the load warning.
    pub fn summary(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        if !self.scene.is_empty() {
            parts.push(format!(
                "{} scene fields ({})",
                self.scene.len(),
                self.scene.keys().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        if !self.nodes.is_empty() {
            parts.push(format!("fields on {} nodes", self.nodes.len()));
        }
        if !self.members.is_empty() {
            parts.push(format!("{} archive members", self.members.len()));
        }
        if !self.settings.is_empty() {
            parts.push(format!("settings ({})", self.settings.keys().cloned().collect::<Vec<_>>().join(", ")));
        }
        if !self.materials.is_empty() {
            parts.push(format!("parts of {} materials", self.materials.len()));
        }
        Some(format!(
            "This file was written by a newer version of Tequila. \
             {} were not understood and will be preserved unchanged when you save.",
            parts.join(", ")
        ))
    }
}

/// A document as loaded, with whatever came with it.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    pub document: SceneDocument,
    pub preserved: Preserved,
    /// Migrations applied and unknown data found, for the log §4.4 asks for.
    pub notes: Vec<String>,
}

// --- writing -------------------------------------------------------------

pub fn save(
    path: &Path,
    document: &SceneDocument,
    preserved: &Preserved,
) -> Result<(), FormatError> {
    let archive = archive(document, preserved);
    if let Some(why) = archive.too_big() {
        return Err(FormatError::Io(format!("{} was not saved: {why}", path.display())));
    }
    let bytes = archive.finish();
    // Written to a temporary alongside the target and renamed, so a crash
    // mid-write leaves the previous save intact rather than a half file where
    // the user's project used to be. And on the disk before the rename: a
    // power cut just after it could otherwise leave the new name on a file
    // whose contents never left the cache, with the old save already gone.
    let temporary = path.with_extension("tequila.saving");
    let written = std::fs::File::create(&temporary).and_then(|mut file| {
        use std::io::Write;
        file.write_all(&bytes)?;
        file.sync_all()
    });
    written.map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        FormatError::Io(format!("{} could not be written: {e}", temporary.display()))
    })?;
    std::fs::rename(&temporary, path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        FormatError::Io(format!("{} could not be replaced: {e}", path.display()))
    })
}

pub fn write(document: &SceneDocument, preserved: &Preserved) -> Vec<u8> {
    archive(document, preserved).finish()
}

/// The project's members, in the order they are written.
fn archive(document: &SceneDocument, preserved: &Preserved) -> zip::ZipWriter {
    let mut archive = zip::ZipWriter::new();

    // Fixed member order, so two saves of one document agree byte for byte.
    archive.add(MANIFEST_MEMBER, write_manifest(document).into_bytes());
    archive.add(SCENE_MEMBER, write_scene(document, preserved).into_bytes());

    for (key, definition) in document.materials.iter() {
        let mut value = material_to_json(key, definition);
        if let (Some(extras), Value::Object(map)) = (preserved.materials.get(&key.0), &mut value) {
            for (k, v) in &extras.unknown {
                map.entry(k.clone()).or_insert_with(|| v.clone());
            }
            if extras.loaded.as_ref() == Some(definition) {
                if let Some(kind) = &extras.original_type {
                    map.insert("type".into(), Value::String(kind.clone()));
                }
                if let Some(graph) = &extras.original_graph {
                    map.insert("graph".into(), graph.clone());
                }
            }
        }
        archive.add(format!("materials/{}.json", material_id(key)), json_write::to_string(&value).into_bytes());
    }
    for (key, mesh) in document.geometry.iter() {
        archive.add(
            format!("geometry/{}.fsmesh", mesh_id(key)),
            fsmesh::write(mesh),
        );
    }
    // CAD sources some model still refers to, and only those: one removed
    // from the scene is not carried in the file for ever.
    let referenced: std::collections::BTreeSet<&str> = document
        .tree
        .iter()
        .filter_map(|n| n.metadata.get(tequila_scene::document::BREP).map(String::as_str))
        .collect();
    for (id, bytes) in document.sources.iter() {
        if referenced.contains(id.as_str()) {
            // Named for what it is: a STEP file begins by saying so.
            let extension = if bytes.trim_ascii_start().starts_with(b"ISO-10303-21") { "step" } else { "iges" };
            archive.add(format!("sources/{id}.{extension}"), bytes.to_vec());
        }
    }
    // Members from a newer build, last so they cannot displace ours.
    for (name, bytes) in &preserved.members {
        archive.add(name.clone(), bytes.clone());
    }
    archive
}

fn write_manifest(document: &SceneDocument) -> String {
    let mut manifest = Object::new();
    manifest
        .set("format_version", document.meta.format_version.clone())
        .set(
            "application_version",
            document.meta.application_version.clone(),
        )
        .set(
            "geometry",
            document
                .geometry
                .iter()
                .map(|(key, _)| Value::from(mesh_id(key)))
                .collect::<Vec<_>>(),
        )
        .set(
            "materials",
            document
                .materials
                .iter()
                .map(|(key, _)| Value::from(material_id(key)))
                .collect::<Vec<_>>(),
        );
    json_write::to_string(&manifest.build())
}

/// Scene metadata in the same schema as the saved project (no mesh buffers).
/// Intended for inspection; edits still go through the command bus.
pub fn scene_description(document: &SceneDocument) -> String {
    write_scene(document, &Preserved::default())
}

fn write_scene(document: &SceneDocument, preserved: &Preserved) -> String {
    let mut scene = Object::new();

    // Unknown fields first, so a known key always wins over a preserved one of
    // the same name — this build's understanding of a field it knows is
    // authoritative.
    let mut root = BTreeMap::new();
    for (key, value) in &preserved.scene {
        root.insert(key.clone(), value.clone());
    }

    scene
        .set("format_version", document.meta.format_version.clone())
        .set("name", document.meta.name.clone())
        .set("units", units_to_json(document.units))
        .set("lighting", lighting_to_json(&document.lighting))
        .set("environment", environment_to_json(&document.environment))
        .set("cameras", cameras_to_json(&document.cameras))
        .set("active_camera", document.cameras.active_index() as f64)
        .set("image_style", image_style_to_json(&document.image_style))
        .set("color_management", color_management_to_json(document.ocio.as_ref()))
        .set("render", render_settings_to_json(&document.render));
    // Each omitted when empty, so a project from before them and one without
    // any are the same bytes.
    if !document.multi_materials.is_empty() {
        scene.set(
            "multi_materials",
            document.multi_materials.iter().map(multi_material_to_json).collect::<Vec<_>>(),
        );
    }
    if document.model_sets != tequila_scene::studio::ModelSets::default() {
        scene.set("model_sets", model_sets_to_json(&document.model_sets));
    }
    if !document.studios.is_empty() {
        scene.set("studios", document.studios.iter().map(studio_to_json).collect::<Vec<_>>());
    }
    // Only once a Studio has turned the product.
    if document.product_turn != tequila_scene::studio::ProductTurn::default() {
        let turn = &document.product_turn;
        let mut object = Object::new();
        object.set("yaw", turn.yaw).set("pivot", Value::Array(vec![Value::from(turn.pivot[0]), Value::from(turn.pivot[1])]));
        scene.set("product_turn", object.build());
    }
    if let Some(animation) = &document.animation {
        scene.set("animation", animation_to_json(animation));
    }
    if document.configurator != tequila_scene::configurator::Configurator::default() {
        scene.set("configurator", configurator_to_json(&document.configurator));
    }
    if !document.material_templates.is_empty() {
        scene.set(
            "material_templates",
            document.material_templates.iter().map(template_to_json).collect::<Vec<_>>(),
        );
    }
    if !document.measurements.is_empty() {
        scene.set(
            "measurements",
            document.measurements.iter().map(measurement_to_json).collect::<Vec<_>>(),
        );
    }
    scene
        .set(
            "tree",
            document
                .tree
                .depth_first()
                .into_iter()
                .filter_map(|id| document.tree.get(id))
                .map(|node| node_to_json(node, preserved))
                .collect::<Vec<_>>(),
        )
        .set(
            "roots",
            document
                .tree
                .roots()
                .iter()
                .map(|id| Value::from(id.0))
                .collect::<Vec<_>>(),
        )
        // The allocator watermarks. Not derivable from the content: a document
        // whose highest node was deleted must still not reissue that ID after
        // a reopen (§2.3).
        // Textures live here rather than in a file each: a reference is a
        // path and a role, and one file apiece would be noise in the archive
        // without making anything more diffable.
        .set(
            "textures",
            document
                .textures
                .iter()
                .map(|(id, source)| {
                    let mut object = Object::new();
                    object
                        .set("id", id.0 as f64)
                        .set("path", source.path.to_string_lossy().to_string())
                        .set("role", journal::role_name(source.role));
                    if let Some(space) = source.color_space {
                        object.set("color_space", journal::color_space_name(space));
                    }
                    object.build()
                })
                .collect::<Vec<_>>(),
        )
        .set("next_node_id", document.tree.next_id())
        .set("next_mesh_key", document.geometry.next_key())
        .set("next_material_key", document.materials.next_key())
        .set("next_texture_id", document.textures.next_id() as f64);

    let Value::Object(mut known) = scene.build() else {
        unreachable!("Object::build returns an object")
    };
    // A newer build's keys inside the settings, where this build's own
    // keys are not: its understanding of a key it knows wins.
    for (path, extra) in &preserved.settings {
        if let Some(Value::Object(object)) = object_at(&mut known, path) {
            for (key, value) in extra {
                object.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
    }
    root.extend(known);
    json_write::to_string(&Value::Object(root))
}

/// The object at a dotted path in `root`, if there is one.
fn object_at<'a>(root: &'a mut BTreeMap<String, Value>, path: &str) -> Option<&'a mut Value> {
    let mut parts = path.split('.');
    let mut current = root.get_mut(parts.next()?)?;
    for part in parts {
        match current {
            Value::Object(map) => current = map.get_mut(part)?,
            _ => return None,
        }
    }
    Some(current)
}

/// The keys a newer build put in the settings objects that this one does
/// not write: what [`write_scene`] would write for `document` is what this
/// build knows, each optional key being written as null when empty, but for
/// [`known_unwritten`].
fn unknown_settings(root: &Value, document: &SceneDocument) -> BTreeMap<String, BTreeMap<String, Value>> {
    let written = json::parse(write_scene(document, &Preserved::default()).as_bytes()).unwrap_or(Value::Null);
    let Value::Object(mut written) = written else { return BTreeMap::new() };
    let Value::Object(file) = root.clone() else { return BTreeMap::new() };
    let mut file = file;
    let mut out = BTreeMap::new();
    for path in SETTINGS_PATHS {
        let known: Vec<String> = match object_at(&mut written, path) {
            Some(Value::Object(map)) => map.keys().cloned().collect(),
            _ => Vec::new(),
        };
        let Some(Value::Object(theirs)) = object_at(&mut file, path) else { continue };
        let unknown: BTreeMap<String, Value> = theirs
            .iter()
            .filter(|(key, _)| !known.contains(key) && !known_unwritten(path).contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if !unknown.is_empty() {
            out.insert(path.to_string(), unknown);
        }
    }
    out
}

fn node_to_json(node: &SceneNode, preserved: &Preserved) -> Value {
    let mut out = BTreeMap::new();
    if let Some(extra) = preserved.nodes.get(&node.id.0) {
        out.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
    }

    let mut object = Object::new();
    object
        .set("id", node.id.0)
        .set("name", node.name.clone())
        .set("kind", node.kind.label())
        .set_some("parent", node.parent.map(|p| p.0))
        .set("transform", matrix_to_json(node.transform))
        .set("visible", node.visible)
        .set("locked", node.locked)
        .set_some("rays", (!node.rays.is_default()).then(|| rays_to_json(&node.rays)))
        .set(
            "children",
            node.children
                .iter()
                .map(|c| Value::from(c.0))
                .collect::<Vec<_>>(),
        );
    match &node.kind {
        NodeKind::Part { mesh } => {
            object.set("mesh", mesh_id(*mesh));
        }
        NodeKind::Light { settings } => {
            object.set("light", light_to_json(settings));
        }
        NodeKind::Cutaway { settings } => {
            object.set("cutaway", cutaway_to_json(settings));
        }
        NodeKind::Group => {}
    }
    object.set_some("material", node.material.map(material_id));
    if !node.metadata.is_empty() {
        object.set(
            "metadata",
            Value::Object(
                node.metadata
                    .iter()
                    .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                    .collect(),
            ),
        );
    }
    // §2.3's persistent path key, which re-import matches on to keep
    // material assignments across an updated CAD file. Useless if it does not
    // survive a save.
    if let Some(key) = node.path_key.as_ref() {
        object.set("path_key", key.clone());
    }
    // §04 §5: "pivots persist for animation use". A pivot that lived only
    // in the gizmo would be lost on save, which is the one thing the word
    // persist rules out.
    if let Some(pivot) = node.pivot {
        object.set("pivot", vec![pivot.x, pivot.y, pivot.z]);
    }

    let Value::Object(known) = object.build() else {
        unreachable!()
    };
    out.extend(known);
    Value::Object(out)
}

/// A node's transform as its 4x4 matrix, in row-major order.
///
/// **Deviation from §4.2**, which sketches `{"t": [...], "r": [...], "s": [...]}`.
/// A TRS triple cannot represent every matrix — a shear has no decomposition
/// into one, and imported CAD assemblies do contain non-uniform scales in
/// nested frames whose product shears. Decomposing would be lossy, and a lossy
/// transform breaks the byte-identical round trip and, worse, moves the user's
/// geometry.
///
/// The inverse is not stored: it is derived on load, so a hand-edited matrix
/// cannot disagree with its own inverse.
fn matrix_to_json(transform: Transform) -> Value {
    let mut values = Vec::with_capacity(16);
    for row in transform.matrix.m {
        for column in row {
            values.push(column);
        }
    }
    Value::from(values)
}

fn units_to_json(units: SceneUnits) -> Value {
    let mut object = Object::new();
    // Lengths are written in metres. The factor is still written, as 1, so
    // a build from before that rule reads them as metres too.
    object
        .set("unit", unit_name(units.unit))
        .set("metres_per_unit", 1.0);
    object.build()
}

/// The environment, written field by field through the reflection table.
///
/// Driven by [`EnvParam::ALL`] rather than by a hand-written list, so a new
/// control on the Environment tab is saved and loaded without either function
/// being touched. `every_environment_field_survives_a_round_trip` is what
/// makes that claim checkable rather than hopeful.
/// One pin, as it is written to a project file.
///
/// Spelled out field by field rather than derived, like everything else in
/// this module: a rename in the struct must not silently rename a key in every
/// file people have already saved.
/// A Material Template, as written to a project file.
pub(crate) fn template_to_json(template: &tequila_scene::templates::MaterialTemplate) -> Value {
    use tequila_scene::templates::Condition;
    let rules: Vec<Value> = template
        .rules
        .iter()
        .map(|rule| {
            let mut object = Object::new();
            match &rule.condition {
                Condition::Name(pattern) => {
                    object.set("match", "name").set("pattern", pattern.clone());
                }
                Condition::Material(pattern) => {
                    object.set("match", "material").set("pattern", pattern.clone());
                }
                Condition::Colour { color: c, tolerance } => {
                    object.set("match", "colour").set("color", color(*c)).set("tolerance", *tolerance);
                }
                Condition::Metadata { key, value } => {
                    object.set("match", "metadata").set("key", key.clone()).set("pattern", value.clone());
                }
            }
            object.set("material", material_id(rule.material));
            object.build()
        })
        .collect();
    let mut object = Object::new();
    object.set("name", template.name.clone()).set("rules", rules);
    object.build()
}

pub(crate) fn read_template(value: &Value) -> tequila_scene::templates::MaterialTemplate {
    use tequila_scene::templates::{Condition, MaterialTemplate, Rule};
    let text = |entry: &Value, key: &str| entry.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    MaterialTemplate {
        name: text(value, "name"),
        // A rule this build cannot read is dropped rather than refusing the
        // file: the rest of the template still works.
        rules: value
            .get("rules")
            .map(|v| {
                v.as_array()
                    .iter()
                    .filter_map(|entry| {
                        let material = entry.get("material")?.as_str().and_then(|s| parse_id("mat_", s))?;
                        let condition = match entry.get("match")?.as_str()? {
                            "name" => Condition::Name(text(entry, "pattern")),
                            "material" => Condition::Material(text(entry, "pattern")),
                            "colour" => Condition::Colour {
                                color: entry.get("color").and_then(read_spectrum)?,
                                tolerance: entry.get("tolerance").and_then(|v| v.as_f32()).unwrap_or(0.02),
                            },
                            "metadata" => Condition::Metadata {
                                key: text(entry, "key"),
                                value: text(entry, "pattern"),
                            },
                            _ => return None,
                        };
                        Some(Rule {
                            condition,
                            material: MaterialKey(material),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// A Multi-Material, as written to a project file.
pub(crate) fn multi_material_to_json(multi: &tequila_scene::studio::MultiMaterial) -> Value {
    let mut object = Object::new();
    object
        .set("id", multi.id)
        .set("name", multi.name.clone())
        .set(
            "variants",
            multi.variants.iter().map(|k| Value::from(material_id(*k))).collect::<Vec<_>>(),
        )
        .set("active", multi.active as f64)
        .set("parts", multi.parts.iter().map(|n| Value::from(n.0)).collect::<Vec<_>>());
    object.build()
}

pub(crate) fn read_multi_material(value: &Value) -> Option<tequila_scene::studio::MultiMaterial> {
    Some(tequila_scene::studio::MultiMaterial {
        id: value.get("id")?.as_index()? as u64,
        name: value.get("name").and_then(|v| v.as_str()).unwrap_or("Multi-Material").to_string(),
        // A material by its file id, "mat_00000003", or by the number every
        // command and query uses for it, 3.
        variants: value
            .get("variants")
            .map(|v| {
                v.as_array()
                    .iter()
                    .filter_map(|k| k.as_str().and_then(|s| parse_id("mat_", s)).or_else(|| k.as_index().map(|i| i as u64)))
                    .map(MaterialKey)
                    .collect()
            })
            .unwrap_or_default(),
        active: value.get("active").and_then(|v| v.as_index()).unwrap_or(0),
        parts: node_list(value.get("parts")),
    })
}

fn node_list(value: Option<&Value>) -> Vec<NodeId> {
    value
        .map(|v| v.as_array().iter().filter_map(|n| n.as_index()).map(|n| NodeId(n as u64)).collect())
        .unwrap_or_default()
}

pub(crate) fn model_sets_to_json(sets: &tequila_scene::studio::ModelSets) -> Value {
    let mut object = Object::new();
    object
        .set(
            "sets",
            sets.sets
                .iter()
                .map(|set| {
                    let mut entry = Object::new();
                    entry
                        .set("name", set.name.clone())
                        .set("nodes", set.nodes.iter().map(|n| Value::from(n.0)).collect::<Vec<_>>())
                        .set("always_visible", set.always_visible);
                    entry.build()
                })
                .collect::<Vec<_>>(),
        )
        .set_some("active", sets.active.map(|i| i as f64));
    object.build()
}

pub(crate) fn read_model_sets(value: &Value) -> tequila_scene::studio::ModelSets {
    let sets: Vec<tequila_scene::studio::ModelSet> = value
        .get("sets")
        .map(|v| {
            v.as_array()
                .iter()
                .map(|entry| tequila_scene::studio::ModelSet {
                    name: entry.get("name").and_then(|v| v.as_str()).unwrap_or("Model Set").to_string(),
                    nodes: node_list(entry.get("nodes")),
                    always_visible: entry.get("always_visible").and_then(|v| v.as_bool()).unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();
    let active = value
        .get("active")
        .and_then(|v| v.as_index())
        .filter(|&i| i < sets.len());
    tequila_scene::studio::ModelSets { sets, active }
}

pub(crate) fn studio_to_json(studio: &tequila_scene::studio::Studio) -> Value {
    let mut object = Object::new();
    object.set("name", studio.name.clone());
    if let Some(camera) = &studio.camera {
        object.set("camera", camera_to_json(camera));
    }
    if let Some(environment) = &studio.environment {
        object.set("environment", environment_to_json(environment));
    }
    if let Some(style) = &studio.image_style {
        object.set("image_style", image_style_to_json(style));
    }
    if let Some(set) = studio.model_set {
        // Null for "show everything", which is a choice; absent for "leave
        // it alone", which is not.
        object.set(
            "model_set",
            match set {
                Some(i) => Value::from(i as f64),
                None => Value::Null,
            },
        );
    }
    object.set(
        "variants",
        studio
            .variants
            .iter()
            .map(|(id, active)| {
                let mut entry = Object::new();
                entry.set("multi_material", *id).set("active", *active as f64);
                entry.build()
            })
            .collect::<Vec<_>>(),
    );
    // Written only when there are some, so a Studio from before views is the
    // same bytes it always was.
    if !studio.lights.is_empty() {
        object.set(
            "lights",
            studio
                .lights
                .iter()
                .map(|o| {
                    let mut entry = Object::new();
                    match &o.target {
                        tequila_scene::studio::LightTarget::Name(name) => entry.set("name", name.clone()),
                        tequila_scene::studio::LightTarget::Layer(layer) => entry.set("layer", f64::from(*layer)),
                    };
                    entry.set("enabled", o.enabled);
                    entry.build()
                })
                .collect::<Vec<_>>(),
        );
    }
    object.set_some("product_yaw", studio.product_yaw);
    object.set_some("filename_suffix", studio.filename_suffix.clone());
    object.build()
}

/// A Studio's light switches, as [`studio_to_json`] writes them.
fn read_light_overrides(value: Option<&Value>) -> Vec<tequila_scene::studio::LightOverride> {
    use tequila_scene::studio::{LightOverride, LightTarget};
    value
        .map(|v| {
            v.as_array()
                .iter()
                .filter_map(|entry| {
                    let target = match (entry.get("name").and_then(|n| n.as_str()), entry.get("layer").and_then(|l| l.as_index())) {
                        (Some(name), _) => LightTarget::Name(name.to_string()),
                        (None, Some(layer)) => LightTarget::Layer(layer.min(255) as u8),
                        (None, None) => return None,
                    };
                    Some(LightOverride { target, enabled: entry.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true) })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn read_studio(value: &Value) -> tequila_scene::studio::Studio {
    tequila_scene::studio::Studio {
        name: value.get("name").and_then(|v| v.as_str()).unwrap_or("Studio").to_string(),
        camera: value.get("camera").map(read_camera),
        // The readers take the object that holds the field, as the scene's
        // top level does.
        environment: value.get("environment").map(|_| read_environment(value)),
        image_style: value.get("image_style").map(read_image_style_value),
        model_set: value.get("model_set").map(|v| v.as_index()),
        variants: value
            .get("variants")
            .map(|v| {
                v.as_array()
                    .iter()
                    .filter_map(|entry| {
                        Some((
                            entry.get("multi_material")?.as_index()? as u64,
                            entry.get("active")?.as_index()?,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        lights: read_light_overrides(value.get("lights")),
        product_yaw: value.get("product_yaw").and_then(|v| v.as_f32()).filter(|v| v.is_finite()),
        filename_suffix: value.get("filename_suffix").and_then(|v| v.as_str()).map(str::to_string),
    }
}

/// The configurator, as written to a project file.
pub(crate) fn configurator_to_json(c: &tequila_scene::configurator::Configurator) -> Value {
    use tequila_scene::configurator::{Drives, RuleKind};
    let options: Vec<Value> = c
        .options
        .iter()
        .map(|option| {
            let mut object = Object::new();
            object.set("id", option.id).set("name", option.name.clone()).set("style", option.style.key());
            match option.drives {
                Drives::Material { multi_material } => {
                    object.set("drives", "material").set("multi_material", multi_material);
                }
                Drives::ModelSet => {
                    object.set("drives", "model_set");
                }
                Drives::Studio => {
                    object.set("drives", "studio");
                }
            }
            let choices: Vec<Value> = option
                .choices
                .iter()
                .map(|choice| {
                    let mut entry = Object::new();
                    entry.set("label", choice.label.clone()).set("target", choice.target);
                    if let Some(swatch) = choice.swatch {
                        entry.set("swatch", swatch.to_vec());
                    }
                    entry.build()
                })
                .collect();
            object.set("choices", choices);
            object.build()
        })
        .collect();
    let rules: Vec<Value> = c
        .rules
        .iter()
        .map(|rule| {
            let mut object = Object::new();
            object
                .set("option", rule.option)
                .set("choice", rule.choice)
                .set("kind", match rule.kind {
                    RuleKind::Excludes => "excludes",
                    RuleKind::Requires => "requires",
                })
                .set("other", rule.other)
                .set("other_choice", rule.other_choice);
            object.build()
        })
        .collect();
    let mut object = Object::new();
    object
        .set("title", c.title.clone())
        .set("subtitle", c.subtitle.clone())
        .set("placement", c.placement.key())
        .set("options", options)
        .set("rules", rules)
        .set("current", c.current.iter().map(|&i| Value::from(i)).collect::<Vec<_>>());
    object.build()
}

/// The configurator from a project file. An option or rule this build
/// cannot read is dropped rather than refusing the file.
pub(crate) fn read_configurator(value: &Value) -> tequila_scene::configurator::Configurator {
    use tequila_scene::configurator::{Choice, ConfigOption, Configurator, Drives, Placement, Rule, RuleKind, Style};
    let text = |entry: &Value, key: &str| entry.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let index = |entry: &Value, key: &str| entry.get(key).and_then(|v| v.as_index());
    let options = value
        .get("options")
        .map(|v| {
            v.as_array()
                .iter()
                .filter_map(|entry| {
                    let drives = match entry.get("drives")?.as_str()? {
                        "material" => Drives::Material { multi_material: index(entry, "multi_material")? as u64 },
                        "model_set" => Drives::ModelSet,
                        "studio" => Drives::Studio,
                        _ => return None,
                    };
                    let choices = entry
                        .get("choices")
                        .map(|v| {
                            v.as_array()
                                .iter()
                                .filter_map(|choice| {
                                    Some(Choice {
                                        label: text(choice, "label"),
                                        target: index(choice, "target")?,
                                        // Linear sRGB, as the panel's picker
                                        // keeps it; or hex, as a person writes it.
                                        swatch: choice.get("swatch").and_then(|s| match s.as_str() {
                                            Some(hex) => hex_colour(hex).map(|c| {
                                                let rgb = c.to_linear_srgb();
                                                [rgb.x.max(0.0), rgb.y.max(0.0), rgb.z.max(0.0)]
                                            }),
                                            None => s.as_f32_array().filter(|s| s.len() == 3).map(|s| [s[0], s[1], s[2]]),
                                        }),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(ConfigOption {
                        id: index(entry, "id")? as u64,
                        name: text(entry, "name"),
                        drives,
                        choices,
                        style: entry.get("style").and_then(|s| s.as_str()).and_then(Style::from_key).unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let rules = value
        .get("rules")
        .map(|v| {
            v.as_array()
                .iter()
                .filter_map(|entry| {
                    Some(Rule {
                        option: index(entry, "option")? as u64,
                        choice: index(entry, "choice")?,
                        kind: match entry.get("kind")?.as_str()? {
                            "excludes" => RuleKind::Excludes,
                            "requires" => RuleKind::Requires,
                            _ => return None,
                        },
                        other: index(entry, "other")? as u64,
                        other_choice: index(entry, "other_choice")?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Configurator {
        title: text(value, "title"),
        subtitle: text(value, "subtitle"),
        placement: value.get("placement").and_then(|s| s.as_str()).and_then(Placement::from_key).unwrap_or_default(),
        options,
        rules,
        current: value
            .get("current")
            .map(|v| v.as_array().iter().filter_map(|i| i.as_index()).collect())
            .unwrap_or_default(),
    }
}

/// One measurement, as written to a project file.
pub(crate) fn measurement_to_json(measurement: &tequila_scene::measure::Measurement) -> Value {
    let anchor = |anchor: &tequila_scene::measure::Anchor| {
        let mut object = Object::new();
        object
            .set_some("node", anchor.node.map(|n| n.0))
            .set("point", Value::from(vec![anchor.point.x, anchor.point.y, anchor.point.z]));
        object.build()
    };
    let mut object = Object::new();
    object
        .set("a", anchor(&measurement.a))
        .set("b", anchor(&measurement.b))
        .set("visible", measurement.visible);
    object.build()
}

pub(crate) fn read_measurement(value: &Value) -> Option<tequila_scene::measure::Measurement> {
    let anchor = |key: &str| {
        let entry = value.get(key)?;
        let point = entry.get("point")?.as_f32_array()?;
        if point.len() < 3 || !point.iter().all(|v| v.is_finite()) {
            return None;
        }
        Some(tequila_scene::measure::Anchor {
            node: entry
                .get("node")
                .and_then(|v| v.as_index())
                .map(|n| tequila_scene::tree::NodeId(n as u64)),
            point: tequila_core::math::vec3(point[0], point[1], point[2]),
        })
    };
    Some(tequila_scene::measure::Measurement {
        a: anchor("a")?,
        b: anchor("b")?,
        visible: value.get("visible").and_then(|v| v.as_bool()).unwrap_or(true),
    })
}

/// A cutaway node's settings, as written to a project file.
pub(crate) fn cutaway_to_json(settings: &tequila_scene::cutaway::CutawaySettings) -> Value {
    use tequila_scene::cutaway::CapStyle;
    let mut object = Object::new();
    object
        .set("shape", settings.shape.key())
        .set("enabled", settings.enabled);
    match settings.cap {
        CapStyle::Own => {
            object.set("cap", "own");
        }
        CapStyle::Color(c) => {
            object.set("cap", "color").set("cap_color", color(c));
        }
    }
    object.build()
}

pub(crate) fn read_cutaway(value: &Value) -> tequila_scene::cutaway::CutawaySettings {
    use tequila_scene::cutaway::{CapStyle, CutShape, CutawaySettings};
    let default = CutawaySettings::default();
    CutawaySettings {
        // An unknown shape reads as a plane rather than refusing the file.
        shape: value
            .get("shape")
            .and_then(|v| v.as_str())
            .and_then(CutShape::from_key)
            .unwrap_or(default.shape),
        cap: match value.get("cap").and_then(|v| v.as_str()) {
            Some("color") => CapStyle::Color(
                value
                    .get("cap_color")
                    .and_then(read_spectrum)
                    .unwrap_or(Spectrum::splat(0.8)),
            ),
            _ => CapStyle::Own,
        },
        enabled: value
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(default.enabled),
    }
}

pub(crate) fn pin_to_json(pin: &tequila_render_core::pins::Pin) -> Value {
    use tequila_render_core::pins::{PinBlend, PinKind, PinShape};
    let mut object = Object::new();
    object.set("name", pin.name.clone());
    // What it paints. Files from before image and gradient pins have no
    // `kind`, and read as lights.
    match &pin.kind {
        PinKind::Light => {
            object.set("kind", "light");
        }
        PinKind::Image { path } => {
            object.set("kind", "image").set("image", path.clone());
        }
        PinKind::Gradient { stops, radial } => {
            let stops: Vec<Value> = stops
                .iter()
                .map(|stop| {
                    let mut entry = Object::new();
                    entry.set("t", stop.t).set("color", color(stop.color));
                    entry.build()
                })
                .collect();
            object
                .set("kind", "gradient")
                .set("stops", Value::Array(stops))
                .set("radial", *radial);
        }
    }
    object
        .set(
            "blend",
            match pin.blend {
                PinBlend::Add => "add",
                PinBlend::Over => "over",
            },
        )
        .set("lighting", pin.in_lighting)
        .set(
            "shape",
            match pin.shape {
                PinShape::Rectangle => "rectangle",
                PinShape::Ellipse => "ellipse",
            },
        )
        .set(
            "direction",
            Value::from(vec![pin.direction.x, pin.direction.y, pin.direction.z]),
        )
        .set(
            "angular_size",
            Value::from(vec![pin.angular_size.x, pin.angular_size.y]),
        )
        .set("roll", pin.roll)
        .set("softness", pin.softness)
        .set("intensity", pin.intensity)
        .set("color", color(pin.color))
        .set("show_in_background", pin.show_in_background)
        .set("enabled", pin.enabled);
    object.build()
}

pub(crate) fn pin_from_json(value: &Value) -> tequila_render_core::pins::Pin {
    use tequila_render_core::pins::{GradientStop, Pin, PinBlend, PinKind, PinShape};
    let default = Pin::default();
    let number = |key: &str, fallback: f32| {
        value
            .get(key)
            .and_then(|v| v.as_f32())
            .filter(|v| v.is_finite())
            .unwrap_or(fallback)
    };
    let floats = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_f32_array())
            .unwrap_or_default()
    };
    let direction = floats("direction");
    let size = floats("angular_size");
    // An unknown kind reads as a light rather than refusing the file: a pin
    // from a newer version still lights the scene, where it is.
    let kind = match value.get("kind").and_then(|v| v.as_str()) {
        Some("image") => PinKind::Image {
            path: value
                .get("image")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        },
        Some("gradient") => PinKind::Gradient {
            stops: value
                .get("stops")
                .map(|v| v.as_array())
                .map(|stops| {
                    stops
                        .iter()
                        .filter_map(|stop| {
                            Some(GradientStop {
                                t: stop.get("t")?.as_f32().filter(|t| t.is_finite())?,
                                color: stop.get("color").and_then(read_spectrum)?,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            radial: value
                .get("radial")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        },
        _ => PinKind::Light,
    };
    Pin {
        name: value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&default.name)
            .to_string(),
        kind,
        blend: match value.get("blend").and_then(|v| v.as_str()) {
            Some("over") => PinBlend::Over,
            _ => PinBlend::Add,
        },
        in_lighting: value
            .get("lighting")
            .and_then(|v| v.as_bool())
            .unwrap_or(default.in_lighting),
        shape: match value.get("shape").and_then(|v| v.as_str()) {
            Some("ellipse") => PinShape::Ellipse,
            _ => PinShape::Rectangle,
        },
        direction: tequila_core::math::vec3(
            *direction.first().unwrap_or(&default.direction.x),
            *direction.get(1).unwrap_or(&default.direction.y),
            *direction.get(2).unwrap_or(&default.direction.z),
        ),
        angular_size: tequila_core::math::vec2(
            *size.first().unwrap_or(&default.angular_size.x),
            *size.get(1).unwrap_or(&default.angular_size.y),
        ),
        roll: number("roll", default.roll),
        softness: number("softness", default.softness),
        intensity: number("intensity", default.intensity),
        color: value
            .get("color")
            .and_then(read_spectrum)
            .unwrap_or(default.color),
        show_in_background: value
            .get("show_in_background")
            .and_then(|v| v.as_bool())
            .unwrap_or(default.show_in_background),
        enabled: value
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(default.enabled),
    }
}

fn sky_to_json(sky: &tequila_render_core::sky::Sky) -> Value {
    let mut object = Object::new();
    object
        .set(
            "sun",
            Value::from(vec![
                sky.sun_direction.x,
                sky.sun_direction.y,
                sky.sun_direction.z,
            ]),
        )
        .set("intensity", sky.intensity)
        .set("turbidity", sky.turbidity)
        .set("ground_albedo", sky.ground_albedo)
        .set("sun_disc", sky.sun_disc)
        .set("sun_intensity", sky.sun_intensity);
    object.build()
}

fn sky_from_json(value: &Value) -> tequila_render_core::sky::Sky {
    let default = tequila_render_core::sky::Sky::default();
    let number = |key: &str, fallback: f32| {
        value
            .get(key)
            .and_then(|v| v.as_f32())
            .filter(|v| v.is_finite())
            .unwrap_or(fallback)
    };
    let sun = value
        .get("sun")
        .and_then(|v| v.as_f32_array())
        .unwrap_or_default();
    tequila_render_core::sky::Sky {
        sun_direction: tequila_core::math::vec3(
            *sun.first().unwrap_or(&default.sun_direction.x),
            *sun.get(1).unwrap_or(&default.sun_direction.y),
            *sun.get(2).unwrap_or(&default.sun_direction.z),
        ),
        intensity: number("intensity", default.intensity),
        turbidity: number("turbidity", default.turbidity),
        ground_albedo: number("ground_albedo", default.ground_albedo),
        sun_disc: value
            .get("sun_disc")
            .and_then(|v| v.as_bool())
            .unwrap_or(default.sun_disc),
        sun_intensity: number("sun_intensity", default.sun_intensity),
    }
}

fn environment_to_json(environment: &EnvironmentSettings) -> Value {
    let mut top = Object::new();
    top.set_some("image", environment.image.clone());
    if let Some(sky) = &environment.sky {
        top.set("sky", sky_to_json(sky));
    }
    // Omitted entirely when there are none, so a project made before pins
    // existed and one made after it with no pins are the same bytes.
    if !environment.pins.is_empty() {
        top.set(
            "pins",
            Value::from(
                environment
                    .pins
                    .iter()
                    .map(pin_to_json)
                    .collect::<Vec<_>>(),
            ),
        );
    }
    let mut sections: BTreeMap<&'static str, Object> = BTreeMap::new();

    for param in EnvParam::ALL {
        let value = match param.read(environment) {
            EnvValue::Scalar(v) => Value::from(v),
            EnvValue::Flag(v) => Value::from(v),
            EnvValue::Color(c) => color(c),
            EnvValue::Mode(m) => Value::from(m.label()),
            EnvValue::Choice(c) => Value::from(c),
            EnvValue::Path(p) => match p {
                Some(path) => Value::from(path),
                // Written as null rather than omitted: the absence has to be
                // distinguishable from a field this build does not know, or
                // §4.4's preservation rule would resurrect a backplate the
                // user cleared.
                None => Value::Null,
            },
        };
        let (section, field) = param.json_path();
        if section.is_empty() {
            top.set(field, value);
        } else {
            sections.entry(section).or_default().set(field, value);
        }
    }

    for (name, object) in sections {
        top.set(name, object.build());
    }
    top.build()
}

/// A light node's settings.
///
/// Written in the units the user set them in — lumens, millimetres, degrees —
/// not in the radiance and half-extent vectors the sampler wants. A file that
/// stored the converted values would silently change meaning the day the
/// conversion is corrected, and there would be no way to tell a light that was
/// authored at 1500 lm from one that happened to land there.
/// Which rays see a node: every flag, written only where some is not the
/// default.
pub(crate) fn rays_to_json(rays: &tequila_scene::tree::Rays) -> Value {
    let mut out = Object::new();
    out.set("camera", rays.camera)
        .set("diffuse", rays.diffuse)
        .set("glossy", rays.glossy)
        .set("transmission", rays.transmission)
        .set("shadow", rays.shadow)
        .set("holdout", rays.holdout);
    out.build()
}

/// The rays a node's `rays` names; absent, every ray, no holdout.
pub(crate) fn read_rays(value: Option<&Value>) -> tequila_scene::tree::Rays {
    let default = tequila_scene::tree::Rays::default();
    let Some(value) = value else { return default };
    let flag = |key: &str, fallback: bool| value.get(key).and_then(|v| v.as_bool()).unwrap_or(fallback);
    tequila_scene::tree::Rays {
        camera: flag("camera", default.camera),
        diffuse: flag("diffuse", default.diffuse),
        glossy: flag("glossy", default.glossy),
        transmission: flag("transmission", default.transmission),
        shadow: flag("shadow", default.shadow),
        holdout: flag("holdout", default.holdout),
    }
}

/// A document's animation, as written to a project file.
pub(crate) fn animation_to_json(animation: &tequila_scene::animation::Animation) -> Value {
    let keys: Vec<Value> = animation
        .keys
        .iter()
        .map(|k| {
            let mut key = Object::new();
            key.set("frame", k.frame as f64).set("product_yaw", k.product_yaw);
            key.build()
        })
        .collect();
    let mut out = Object::new();
    out.set("frames", animation.frames as f64)
        .set("fps", animation.fps)
        .set("keys", Value::Array(keys))
        .set("interpolation", animation.interpolation.key());
    out.build()
}

pub(crate) fn read_animation(value: &Value) -> tequila_scene::animation::Animation {
    use tequila_scene::animation::{Animation, Interpolation, TurnKey};
    Animation {
        frames: value.get("frames").and_then(|v| v.as_index()).unwrap_or(1).clamp(1, u32::MAX as usize) as u32,
        fps: value.get("fps").and_then(|v| v.as_f32()).filter(|f| *f > 0.0).unwrap_or(24.0),
        keys: value
            .get("keys")
            .map(|keys| {
                keys.as_array()
                    .iter()
                    .filter_map(|k| {
                        Some(TurnKey {
                            frame: k.get("frame")?.as_index()?.min(u32::MAX as usize) as u32,
                            product_yaw: k.get("product_yaw")?.as_f32().filter(|v| v.is_finite())?,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        interpolation: value.get("interpolation").and_then(|v| v.as_str()).and_then(Interpolation::from_key).unwrap_or_default(),
    }
}

pub(crate) fn light_to_json(settings: &LightSettings) -> Value {
    let mut object = Object::new();
    object.set("shape", settings.shape.label());
    match settings.shape {
        LightShape::Area {
            width,
            height,
            disc: _,
            two_sided,
        } => {
            object
                .set("width", width)
                .set("height", height)
                .set("two_sided", two_sided);
        }
        LightShape::Point { radius } => {
            object.set("radius", radius);
        }
        LightShape::Spot {
            inner_degrees,
            outer_degrees,
            falloff,
        } => {
            object
                .set("inner_degrees", inner_degrees)
                .set("outer_degrees", outer_degrees)
                .set("falloff", falloff);
        }
        LightShape::Sun { angle_degrees } => {
            object.set("angle_degrees", angle_degrees);
        }
    }
    object
        .set("color", color(settings.color))
        .set("intensity", settings.intensity.value())
        .set("intensity_unit", settings.intensity.unit())
        .set("layer", settings.layer as f64)
        .set("enabled", settings.enabled)
        .set("visible_to_camera", settings.visibility.camera)
        .set("affects_diffuse", settings.visibility.diffuse)
        .set("affects_specular", settings.visibility.specular)
        .set("casts_shadows", settings.visibility.shadow);
    if let Some(kelvin) = settings.temperature {
        object.set("temperature", kelvin);
    }
    if let Some(photometry) = &settings.photometry {
        object.set("photometry", photometry.path.clone());
    }
    if let Some(links) = &settings.links {
        object.set("links", linking_to_json(links));
    }
    if let Some(links) = &settings.shadow_links {
        object.set("shadow_links", linking_to_json(links));
    }
    // Only where it is not a plain panel's, so a file from before reads
    // and writes as it was.
    if settings.spread_degrees < 180.0 {
        object.set("spread_degrees", settings.spread_degrees);
    }
    object.build()
}

/// A light's links, for its light (`links`) or its shadow (`shadow_links`).
fn linking_to_json(links: &tequila_scene::lighting::LightLinking) -> Value {
    let mut linking = Object::new();
    linking
        .set("exclude", links.exclude)
        .set("nodes", links.nodes.iter().map(|n| Value::from(n.0 as f64)).collect::<Vec<_>>())
        .set("product", links.product);
    linking.build()
}

fn read_linking(links: &Value) -> tequila_scene::lighting::LightLinking {
    tequila_scene::lighting::LightLinking {
        exclude: links.get("exclude").and_then(|v| v.as_bool()).unwrap_or(false),
        nodes: links
            .get("nodes")
            .map(|v| v.as_array().iter().filter_map(|n| n.as_index()).map(|n| tequila_scene::tree::NodeId(n as u64)).collect())
            .unwrap_or_default(),
        product: links.get("product").and_then(|v| v.as_bool()).unwrap_or(false),
    }
}

pub(crate) fn read_light(value: &Value) -> LightSettings {
    let mut settings = LightSettings::default();
    let number = |key: &str, fallback: f32| value.get(key).and_then(|v| v.as_f32()).unwrap_or(fallback);
    let flag = |key: &str, fallback: bool| value.get(key).and_then(|v| v.as_bool()).unwrap_or(fallback);

    settings.shape = match value.get("shape").and_then(|v| v.as_str()) {
        Some("Point") => LightShape::Point {
            radius: number("radius", 0.0),
        },
        Some("Spot") => LightShape::Spot {
            inner_degrees: number("inner_degrees", 20.0),
            outer_degrees: number("outer_degrees", 35.0),
            falloff: number("falloff", 1.0),
        },
        Some("Sun") => LightShape::Sun {
            angle_degrees: number("angle_degrees", 0.53),
        },
        shape => LightShape::Area {
            width: number("width", 1.0),
            height: number("height", 1.0),
            disc: shape == Some("Disc"),
            two_sided: flag("two_sided", false),
        },
    };
    if let Some(c) = value.get("color").and_then(read_spectrum) {
        settings.color = c;
    }
    let amount = number("intensity", settings.intensity.value());
    settings.intensity = match value.get("intensity_unit").and_then(|v| v.as_str()) {
        Some("nits") => Intensity::Nits(amount),
        Some("cd") => Intensity::Candela(amount),
        Some("lx") => Intensity::Lux(amount),
        _ => Intensity::Lumens(amount),
    };
    settings.temperature = value.get("temperature").and_then(|v| v.as_f32());
    settings.photometry = value
        .get("photometry")
        .and_then(|v| v.as_str())
        .map(|path| tequila_scene::lighting::Photometry {
            path: path.to_string(),
        });
    settings.layer = value
        .get("layer")
        .and_then(|v| v.as_index())
        .unwrap_or(0)
        .min(255) as u8;
    settings.enabled = flag("enabled", true);
    settings.links = value.get("links").map(read_linking);
    settings.shadow_links = value.get("shadow_links").map(read_linking);
    settings.spread_degrees = number("spread_degrees", 180.0).clamp(1.0, 180.0);
    settings.visibility = Visibility {
        camera: flag("visible_to_camera", true),
        diffuse: flag("affects_diffuse", true),
        specular: flag("affects_specular", true),
        shadow: flag("casts_shadows", true),
    };
    settings
}

fn lighting_to_json(lighting: &LightingSettings) -> Value {
    let mut object = Object::new();
    match lighting.preset {
        Some(preset) => object.set("preset", preset_name(preset)),
        // Custom is the absence of a preset, so the absence is what is
        // written. A "Custom" string would be a sixth preset name that has to
        // be special-cased everywhere it is read.
        None => object.set("preset", Value::Null),
    };
    object.set("custom", integrator_to_json(&lighting.custom));
    object.build()
}

pub(crate) fn integrator_to_json(config: &IntegratorConfig) -> Value {
    let mut object = Object::new();
    object
        .set("max_bounces", config.max_bounces as f64)
        .set("global_illumination", config.global_illumination)
        .set("direct_lighting", config.direct_lighting)
        .set("shadows", config.shadows)
        .set("light_source_materials", config.light_source_materials)
        .set("ground_illumination", config.ground_illumination)
        .set("caustics", config.caustics)
        .set("interior_mode", config.interior_mode)
        .set(
            "regularization_start_bounce",
            config.regularization_start_bounce as f64,
        )
        .set("regularization_roughness", config.regularization_roughness)
        .set(
            "adaptive_sampling_threshold",
            config.adaptive_sampling_threshold,
        )
        .set(
            "max_transmission_bounces",
            config.max_transmission_bounces as f64,
        )
        .set(
            "russian_roulette_start_bounce",
            config.russian_roulette_start_bounce as f64,
        );
    // Only when something is set, so a project that sets none is the same
    // bytes it always was.
    if config.cycles != tequila_render_core::settings::CyclesTransport::NONE {
        let c = &config.cycles;
        let mut cycles = Object::new();
        cycles
            .set_some("diffuse_bounces", c.diffuse_bounces.map(f64::from))
            .set_some("glossy_bounces", c.glossy_bounces.map(f64::from))
            .set_some("transparent_bounces", c.transparent_bounces.map(f64::from))
            .set_some("volume_bounces", c.volume_bounces.map(f64::from))
            .set_some("filter_glossy", c.filter_glossy.map(f64::from))
            .set_some("caustics_reflective", c.caustics_reflective)
            .set_some("caustics_refractive", c.caustics_refractive);
        object.set("cycles", cycles.build());
    }
    object.build()
}

fn read_cycles_transport(value: Option<&Value>) -> tequila_render_core::settings::CyclesTransport {
    let Some(value) = value else { return tequila_render_core::settings::CyclesTransport::NONE };
    let count = |key: &str| value.get(key).and_then(|v| v.as_index()).map(|v| v as u32);
    let flag = |key: &str| value.get(key).and_then(|v| v.as_bool());
    tequila_render_core::settings::CyclesTransport {
        diffuse_bounces: count("diffuse_bounces"),
        glossy_bounces: count("glossy_bounces"),
        transparent_bounces: count("transparent_bounces"),
        volume_bounces: count("volume_bounces"),
        filter_glossy: value.get("filter_glossy").and_then(|v| v.as_f32()).map(|v| v.max(0.0)),
        caustics_reflective: flag("caustics_reflective"),
        caustics_refractive: flag("caustics_refractive"),
    }
}

pub(crate) fn read_integrator(value: &Value, default: IntegratorConfig) -> IntegratorConfig {
    let count = |key: &str, fallback: u32| {
        value
            .get(key)
            .and_then(|v| v.as_index())
            .map(|v| v as u32)
            .unwrap_or(fallback)
    };
    let number = |key: &str, fallback: f32| {
        value.get(key).and_then(|v| v.as_f32()).unwrap_or(fallback)
    };
    let flag = |key: &str, fallback: bool| {
        value.get(key).and_then(|v| v.as_bool()).unwrap_or(fallback)
    };
    IntegratorConfig {
        max_bounces: count("max_bounces", default.max_bounces),
        global_illumination: flag("global_illumination", default.global_illumination),
        direct_lighting: flag("direct_lighting", default.direct_lighting),
        shadows: flag("shadows", default.shadows),
        light_source_materials: flag("light_source_materials", default.light_source_materials),
        ground_illumination: flag("ground_illumination", default.ground_illumination),
        caustics: flag("caustics", default.caustics),
        interior_mode: flag("interior_mode", default.interior_mode),
        regularization_start_bounce: count(
            "regularization_start_bounce",
            default.regularization_start_bounce,
        ),
        regularization_roughness: number(
            "regularization_roughness",
            default.regularization_roughness,
        ),
        adaptive_sampling_threshold: number(
            "adaptive_sampling_threshold",
            default.adaptive_sampling_threshold,
        ),
        max_transmission_bounces: count(
            "max_transmission_bounces",
            default.max_transmission_bounces,
        ),
        russian_roulette_start_bounce: count(
            "russian_roulette_start_bounce",
            default.russian_roulette_start_bounce,
        ),
        // Per render, from the camera; not a setting, and not in the file.
        pixel_spread: default.pixel_spread,
        in_floor_reflection: false,
        floor_reflection_only: false,
        cycles: read_cycles_transport(value.get("cycles")),
        // The render's, set per render.
        pixel_filter: default.pixel_filter,
    }
}

fn read_lighting(root: &Value) -> LightingSettings {
    let default = LightingSettings::default();
    let Some(value) = root.get("lighting") else {
        // A document written before the Lighting tab existed carries only the
        // preset name. Reading it keeps those files opening as they did.
        let preset = root
            .get("lighting_preset")
            .and_then(|v| v.as_str())
            .and_then(preset_from_name)
            .unwrap_or(LightingPreset::Product);
        return LightingSettings::from_preset(preset);
    };
    let preset = value
        .get("preset")
        .and_then(|v| v.as_str())
        .and_then(preset_from_name);
    let custom = match value.get("custom") {
        Some(v) => read_integrator(v, default.custom.clone()),
        None => default.custom.clone(),
    };
    LightingSettings { preset, custom }
}

pub(crate) fn image_style_to_json(style: &ImageStyle) -> Value {
    let mut object = Object::new();
    object
        .set("type", style.mode.label())
        .set("exposure", style.exposure)
        .set("contrast", style.contrast)
        .set("saturation", style.saturation)
        .set("highlights", style.highlights)
        .set("shadows", style.shadows)
        .set("whites", style.whites)
        .set("blacks", style.blacks)
        .set("temperature", style.temperature)
        .set("tint", style.tint)
        .set("vignette", style.vignette)
        .set("bloom", style.bloom)
        .set("bloom_radius", style.bloom_radius)
        .set("sharpening", style.sharpening)
        .set("chromatic_aberration", style.chromatic_aberration)
        .set("display_transform", style.display.label());
    object.build()
}

fn read_image_style(root: &Value) -> ImageStyle {
    match root.get("image_style") {
        Some(value) => read_image_style_value(value),
        None => ImageStyle::default(),
    }
}

/// The OCIO display and view, or null when the built-in transforms are in use.
///
/// A path and two names, never the config's contents. A config is a studio's
/// file, shared across projects and updated independently of them; a project
/// that embedded a copy would keep developing through last year's version of
/// it, which is the one thing pointing at a config is supposed to prevent.
/// The render settings (§1.8).
pub(crate) fn render_settings_to_json(settings: &RenderSettings) -> Value {
    let mut object = Object::new();
    object
        .set("engine", settings.engine.key())
        .set("resolution", Value::Array(vec![
            Value::from(settings.width as f64),
            Value::from(settings.height as f64),
        ]))
        .set("dpi", settings.dpi)
        .set("max_samples", settings.max_samples as f64)
        .set("adaptive", settings.adaptive)
        .set("denoise", settings.denoise)
        .set("denoise_strength", settings.denoise_strength)
        .set("denoise_cadence", settings.denoise_cadence as f64)
        .set("prefilter_guides", settings.prefilter_guides)
        .set("reject_fireflies", settings.reject_fireflies)
        .set("format", settings.format.key())
        .set("depth", depth_key(settings.depth))
        .set("alpha_mode", settings.alpha_mode.key())
        .set("jpeg_quality", settings.jpeg_quality as f64)
        .set("write_passes", settings.write_passes)
        .set("output_directory", settings.output_directory.clone())
        .set("filename_template", settings.filename_template.clone())
        .set("write_xmp", settings.write_xmp)
        .set("write_text_metadata", settings.write_text_metadata);
    object.set("relight", settings.relight);
    object.set(
        "layers",
        settings
            .layers
            .iter()
            .map(|layer| {
                let mut entry = Object::new();
                entry
                    .set("name", layer.name.clone())
                    .set("nodes", layer.nodes.iter().map(|n| Value::from(n.0)).collect::<Vec<_>>())
                    .set("enabled", layer.enabled);
                entry.build()
            })
            .collect::<Vec<_>>(),
    );
    // The optional ones are written as null rather than omitted, so "no time
    // limit" is distinguishable from "a field this build does not know" and
    // §4.4's preservation rule cannot resurrect a limit the user cleared.
    match settings.max_seconds {
        Some(seconds) => object.set("max_seconds", seconds as f64),
        None => object.set("max_seconds", Value::Null),
    };
    match settings.clamp_indirect {
        Some(clamp) => object.set("clamp_indirect", clamp),
        None => object.set("clamp_indirect", Value::Null),
    };
    match settings.clamp_direct {
        Some(clamp) => object.set("clamp_direct", clamp),
        None => object.set("clamp_direct", Value::Null),
    };
    match settings.adaptive_threshold {
        Some(threshold) => object.set("adaptive_threshold", threshold),
        None => object.set("adaptive_threshold", Value::Null),
    };
    match settings.pixel_filter {
        Some(filter) => {
            let mut f = Object::new();
            f.set("type", filter.kind.key()).set("width_px", filter.width);
            object.set("pixel_filter", f.build())
        }
        None => object.set("pixel_filter", Value::Null),
    };
    match settings.region {
        Some(region) => object.set(
            "region",
            Value::Array(vec![
                Value::from(region.x as f64),
                Value::from(region.y as f64),
                Value::from(region.width as f64),
                Value::from(region.height as f64),
            ]),
        ),
        None => object.set("region", Value::Null),
    };
    object.build()
}

fn depth_key(depth: BitDepth) -> &'static str {
    match depth {
        BitDepth::Eight => "8",
        BitDepth::Sixteen => "16",
        BitDepth::ThirtyTwo => "32",
    }
}

fn depth_from_key(key: &str) -> Option<BitDepth> {
    match key {
        "8" => Some(BitDepth::Eight),
        "16" => Some(BitDepth::Sixteen),
        "32" => Some(BitDepth::ThirtyTwo),
        _ => None,
    }
}

pub(crate) fn read_render_settings_value(value: &Value) -> RenderSettings {
    let default = RenderSettings::default();
    let number = |key: &str, fallback: f32| value.get(key).and_then(|v| v.as_f32()).unwrap_or(fallback);
    let count = |key: &str, fallback: u32| {
        value
            .get(key)
            .and_then(|v| v.as_index())
            .map(|v| v as u32)
            .unwrap_or(fallback)
    };
    let flag = |key: &str, fallback: bool| value.get(key).and_then(|v| v.as_bool()).unwrap_or(fallback);

    let resolution = value.get("resolution").map(|v| v.as_array()).unwrap_or(&[]);
    let dimension = |index: usize, fallback: usize| {
        resolution
            .get(index)
            .and_then(|v| v.as_index())
            .unwrap_or(fallback)
            .max(1)
    };

    RenderSettings {
        // A project from before there was a choice, or naming an engine this
        // build does not know, renders with the default, Cycles.
        engine: value
            .get("engine")
            .and_then(|v| v.as_str())
            .and_then(tequila_scene::render::RenderEngine::from_key)
            .unwrap_or_default(),
        width: dimension(0, default.width),
        height: dimension(1, default.height),
        dpi: number("dpi", default.dpi),
        max_samples: count("max_samples", default.max_samples).max(1),
        max_seconds: value.get("max_seconds").and_then(|v| v.as_index()).map(|v| v as u32),
        adaptive: flag("adaptive", default.adaptive),
        denoise: flag("denoise", default.denoise),
        denoise_strength: number("denoise_strength", default.denoise_strength),
        denoise_cadence: count("denoise_cadence", default.denoise_cadence).max(1),
        prefilter_guides: flag("prefilter_guides", default.prefilter_guides),
        reject_fireflies: flag("reject_fireflies", default.reject_fireflies),
        clamp_indirect: value.get("clamp_indirect").and_then(|v| v.as_f32()),
        clamp_direct: value.get("clamp_direct").and_then(|v| v.as_f32()).map(|v| v.max(0.0)),
        adaptive_threshold: value.get("adaptive_threshold").and_then(|v| v.as_f32()).filter(|v| *v > 0.0),
        pixel_filter: value.get("pixel_filter").and_then(|f| {
            let kind = tequila_render_core::settings::PixelFilterKind::from_key(f.get("type")?.as_str()?)?;
            let width = f.get("width_px").and_then(|w| w.as_f32()).filter(|w| w.is_finite() && *w > 0.0).unwrap_or(1.0);
            Some(tequila_render_core::settings::PixelFilter { kind, width })
        }),
        region: value.get("region").and_then(|v| {
            let parts = v.as_array();
            if parts.len() < 4 {
                return None;
            }
            let at = |i: usize| parts[i].as_index().unwrap_or(0);
            let region = Region {
                x: at(0),
                y: at(1),
                width: at(2),
                height: at(3),
            };
            // A region of no width renders nothing, which is never what anyone
            // meant; reading it as "no region" is the recoverable answer.
            (region.width > 0 && region.height > 0).then_some(region)
        }),
        format: value
            .get("format")
            .and_then(|v| v.as_str())
            .and_then(OutputFormat::from_key)
            .unwrap_or(default.format),
        depth: value
            .get("depth")
            .and_then(|v| v.as_str())
            .and_then(depth_from_key)
            .unwrap_or(default.depth),
        alpha_mode: value
            .get("alpha_mode")
            .and_then(|v| v.as_str())
            .and_then(SceneAlphaMode::from_key)
            .unwrap_or(default.alpha_mode),
        jpeg_quality: count("jpeg_quality", default.jpeg_quality as u32).clamp(1, 100) as u8,
        write_passes: flag("write_passes", default.write_passes),
        output_directory: value
            .get("output_directory")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        filename_template: value
            .get("filename_template")
            .and_then(|v| v.as_str())
            .unwrap_or(&default.filename_template)
            .to_string(),
        write_xmp: flag("write_xmp", default.write_xmp),
        write_text_metadata: flag("write_text_metadata", default.write_text_metadata),
        relight: flag("relight", default.relight),
        layers: value
            .get("layers")
            .map(|list| {
                list.as_array()
                    .iter()
                    .map(|entry| tequila_scene::render::RenderLayer {
                        name: entry.get("name").and_then(|v| v.as_str()).unwrap_or("Layer").to_string(),
                        nodes: node_list(entry.get("nodes")),
                        enabled: entry.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn read_render_settings(root: &Value) -> RenderSettings {
    match root.get("render") {
        Some(value) => read_render_settings_value(value),
        None => RenderSettings::default(),
    }
}

pub(crate) fn color_management_to_json(selection: Option<&OcioSelection>) -> Value {
    let Some(selection) = selection else {
        return Value::Null;
    };
    let mut object = Object::new();
    object
        .set("config", selection.config.clone())
        .set("display", selection.display.clone())
        .set("view", selection.view.clone());
    object.build()
}

pub(crate) fn read_color_management_value(value: &Value) -> Option<OcioSelection> {
    let config = value.get("config").and_then(|v| v.as_str())?;
    Some(OcioSelection {
        config: config.to_string(),
        display: value
            .get("display")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        view: value
            .get("view")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

fn read_color_management(root: &Value) -> Option<OcioSelection> {
    read_color_management_value(root.get("color_management")?)
}

pub(crate) fn read_image_style_value(value: &Value) -> ImageStyle {
    let default = ImageStyle::default();
    let number = |key: &str, fallback: f32| {
        value.get(key).and_then(|v| v.as_f32()).unwrap_or(fallback)
    };
    ImageStyle {
        mode: value
            .get("type")
            .and_then(|v| v.as_str())
            .and_then(|s| ToneMode::ALL.into_iter().find(|m| m.label() == s))
            .unwrap_or(default.mode),
        exposure: number("exposure", default.exposure),
        contrast: number("contrast", default.contrast),
        saturation: number("saturation", default.saturation),
        highlights: number("highlights", default.highlights),
        shadows: number("shadows", default.shadows),
        whites: number("whites", default.whites),
        blacks: number("blacks", default.blacks),
        temperature: number("temperature", default.temperature),
        tint: number("tint", default.tint),
        vignette: number("vignette", default.vignette),
        bloom: number("bloom", default.bloom),
        bloom_radius: number("bloom_radius", default.bloom_radius),
        sharpening: number("sharpening", default.sharpening),
        chromatic_aberration: number("chromatic_aberration", default.chromatic_aberration),
        display: value
            .get("display_transform")
            .and_then(|v| v.as_str())
            .and_then(|s| DisplayTransform::ALL.into_iter().find(|d| d.label() == s))
            // A file from before the choice was saved was rendered with AgX,
            // the default then, and keeps it rather than changing look.
            .unwrap_or(DisplayTransform::AgX),
    }
}

/// The saved cameras (§4, §8).
///
/// Written as eye/target/up rather than as a matrix, because those three are
/// what a user set and what another tool can read; a matrix would round-trip
/// perfectly and tell a person nothing. The focal length lives inside the
/// projection, since an orthographic camera does not have one and writing a
/// meaningless number for it invites something downstream to read it.
fn cameras_to_json(store: &CameraStore) -> Value {
    Value::Array(store.iter().map(|(_, saved)| camera_to_json(saved)).collect())
}

/// One camera.
///
/// `position`, `target` and `up` are what `docs/03` §4.1 specifies and what a
/// person or another tool can read. They are also a *lossy* encoding of an
/// orthonormal basis: rebuilding one with a cross product and a normalize
/// lands within a few ULPs of where it started, not on it, so a saved camera
/// that was reopened and saved again would not be byte-identical — which M1's
/// exit criterion forbids. `basis` carries the three axes exactly for that
/// reason, and is used when present; the readable triple is the fallback for
/// files written by anything else.
pub(crate) fn camera_to_json(saved: &SavedCamera) -> Value {
    let camera = &saved.camera;
    let m = &camera.camera_to_world;
    let axis = |v: Vec3| [v.x, v.y, v.z];
    let basis: Vec<Value> = axis(m.transform_vector(Vec3::X))
        .into_iter()
        .chain(axis(m.transform_vector(Vec3::Y)))
        .chain(axis(m.transform_vector(Vec3::Z)))
        .map(Value::from)
        .collect();
    let mut object = Object::new();
    object
        .set("name", saved.name.clone())
        .set("position", vector(camera.position()))
        .set("target", vector(saved.target))
        .set("up", vector(m.transform_vector(Vec3::Y)))
        .set("basis", Value::Array(basis))
        .set("type", lens_name(camera.projection))
        .set("sensor_width_mm", camera.sensor_width_mm)
        .set("iso", camera.iso)
        .set("shutter_speed_seconds", camera.shutter_speed_seconds);
    // Written only when on, so a file from before is unchanged.
    if camera.physical_exposure {
        object.set("physical_exposure", true);
    }
    match camera.projection {
        Projection::Perspective { focal_length_mm } => {
            object.set("focal_length_mm", focal_length_mm);
        }
        Projection::Orthographic { scale } => {
            object.set("scale", scale);
        }
        Projection::Shift {
            focal_length_mm,
            shift_x,
            shift_y,
        } => {
            object
                .set("focal_length_mm", focal_length_mm)
                .set("shift_x", shift_x)
                .set("shift_y", shift_y);
        }
        Projection::Equirectangular => {}
    }
    let mut dof = Object::new();
    dof.set("enabled", camera.f_number.is_finite())
        .set(
            "f_number",
            if camera.f_number.is_finite() {
                camera.f_number
            } else {
                // A pinhole is infinity, and JSON has no infinity. The flag
                // above carries the meaning; this is the f-number the control
                // returns to when depth of field is switched back on.
                8.0
            },
        )
        .set("focus_distance", camera.focus_distance)
        .set("keep_focus", camera.keep_focus)
        .set("aperture", aperture_name(camera.aperture));
    if let Aperture::Bladed { blades, rotation } = camera.aperture {
        dof.set("blades", blades as f64).set("rotation", rotation);
    }
    object.set("dof", dof.build());
    object.build()
}

pub(crate) fn read_camera(entry: &Value) -> SavedCamera {
    let eye = entry
        .get("position")
        .and_then(read_vector)
        .unwrap_or(Vec3::ZERO);
    let target = entry
        .get("target")
        .and_then(read_vector)
        .unwrap_or(Vec3::ZERO);
    let up = entry.get("up").and_then(read_vector).unwrap_or(Vec3::Y);
    let mut camera = match entry.get("basis").map(|v| v.as_array()) {
        Some(axes) if axes.len() >= 9 => {
            let f = |i: usize| axes[i].as_f32().unwrap_or(0.0);
            let mut m = Mat4::IDENTITY;
            for (column, base) in [(0, 0), (1, 3), (2, 6)] {
                m.m[0][column] = f(base);
                m.m[1][column] = f(base + 1);
                m.m[2][column] = f(base + 2);
            }
            m.m[0][3] = eye.x;
            m.m[1][3] = eye.y;
            m.m[2][3] = eye.z;
            Camera {
                camera_to_world: m,
                focus_distance: (target - eye).length().max(1e-4),
                ..Default::default()
            }
        }
        _ => Camera::look_at(eye, target, up),
    };

    let number =
        |key: &str, fallback: f32| entry.get(key).and_then(|v| v.as_f32()).unwrap_or(fallback);
    camera.projection = match entry.get("type").and_then(|v| v.as_str()) {
        Some("Orthographic") => Projection::Orthographic {
            scale: number("scale", 100.0),
        },
        Some("Shift") => Projection::Shift {
            focal_length_mm: number("focal_length_mm", 50.0),
            shift_x: number("shift_x", 0.0),
            shift_y: number("shift_y", 0.0),
        },
        Some("Spherical") | Some("Equirectangular") => Projection::Equirectangular,
        _ => Projection::Perspective {
            focal_length_mm: number("focal_length_mm", 50.0),
        },
    };
    camera.sensor_width_mm = number("sensor_width_mm", camera.sensor_width_mm);
    camera.iso = number("iso", camera.iso);
    camera.shutter_speed_seconds = number("shutter_speed_seconds", camera.shutter_speed_seconds);
    camera.physical_exposure = entry.get("physical_exposure").and_then(|v| v.as_bool()).unwrap_or(false);

    if let Some(dof) = entry.get("dof") {
        let enabled = dof.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
        let f_number = dof.get("f_number").and_then(|v| v.as_f32()).unwrap_or(8.0);
        camera.f_number = if enabled { f_number } else { f32::INFINITY };
        if let Some(v) = dof.get("focus_distance").and_then(|v| v.as_f32()) {
            camera.focus_distance = v;
        }
        camera.keep_focus = dof
            .get("keep_focus")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if dof.get("aperture").and_then(|v| v.as_str()) == Some("Bladed") {
            camera.aperture = Aperture::Bladed {
                blades: dof
                    .get("blades")
                    .and_then(|v| v.as_index())
                    .unwrap_or(6)
                    .max(3) as u32,
                rotation: dof.get("rotation").and_then(|v| v.as_f32()).unwrap_or(0.0),
            };
        }
    }

    SavedCamera {
        name: entry
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Camera")
            .to_string(),
        camera,
        target,
    }
}

fn read_cameras(root: &Value) -> CameraStore {
    let Some(entries) = root.get("cameras").map(|v| v.as_array()) else {
        return CameraStore::new();
    };
    let mut store = CameraStore::from_cameras(entries.iter().map(read_camera).collect());
    store.select(
        root.get("active_camera")
            .and_then(|v| v.as_index())
            .unwrap_or(0),
    );
    store
}

fn aperture_name(aperture: Aperture) -> &'static str {
    match aperture {
        Aperture::Circular => "Circular",
        Aperture::Bladed { .. } => "Bladed",
    }
}

/// The name of a lens type, from the one table the Camera tab also reads.
fn lens_name(projection: Projection) -> &'static str {
    let same_kind = |a: &Projection, b: &Projection| {
        std::mem::discriminant(a) == std::mem::discriminant(b)
    };
    tequila_scene::lighting::lens_types()
        .into_iter()
        .find(|(_, p)| same_kind(p, &projection))
        .map(|(label, _)| label)
        .unwrap_or("Perspective")
}

fn vector(v: Vec3) -> Value {
    Value::Array(vec![Value::from(v.x), Value::from(v.y), Value::from(v.z)])
}

fn read_vector(value: &Value) -> Option<Vec3> {
    let parts = value.as_array();
    if parts.len() < 3 {
        return None;
    }
    Some(tequila_core::math::vec3(
        parts[0].as_f32()?,
        parts[1].as_f32()?,
        parts[2].as_f32()?,
    ))
}

/// A material, written parameter by parameter through the reflection table.
///
/// **Every** parameter is written, not only those differing from the type's
/// defaults. Writing only the differences would be smaller, but it would make
/// a saved file's meaning depend on the preset table of whichever version
/// opens it: adjust the Plastic preset in a later release and every Plastic
/// material in every existing file quietly changes. A document has to mean the
/// same thing forever, so it carries its own values.
///
/// The type is still recorded, because it is what the Material tab uses to
/// decide which parameters to show (§04 §3) — an identity, not a shading mode.
pub fn material_to_json(key: MaterialKey, definition: &MaterialDefinition) -> Value {
    let m = definition.material();
    let mut object = Object::new();
    object
        .set("id", material_id(key))
        .set("name", definition.name.clone())
        .set("type", definition.material_type.id());
    for param in Param::ALL {
        match param.read(m) {
            Some(ParamValue::Scalar(v)) => {
                object.set(param.key(), v);
            }
            Some(ParamValue::Color(v)) => {
                object.set(param.key(), color(v));
            }
            Some(ParamValue::Flag(v)) => {
                object.set(param.key(), v);
            }
            Some(ParamValue::Metal(v)) => {
                let mut ior = Object::new();
                ior.set("n", color(v.n)).set("k", color(v.k));
                object.set(param.key(), ior.build());
            }
            // A texture slot has no literal of its own; whatever drives it
            // is a link in the graph below.
            None => {}
        }
    }
    if let Some(graph) = graph::graph_to_json(&definition.graph) {
        object.set("graph", graph);
    }
    object.build()
}

pub(crate) fn color(s: Spectrum) -> Value {
    Value::from(vec![s.r, s.g, s.b])
}

fn mesh_id(key: MeshKey) -> String {
    format!("m_{:08x}", key.0)
}

fn material_id(key: MaterialKey) -> String {
    format!("mat_{:08x}", key.0)
}

pub(crate) fn parse_id(prefix: &str, text: &str) -> Option<u64> {
    u64::from_str_radix(text.strip_prefix(prefix)?, 16).ok()
}

pub(crate) fn unit_name(unit: LengthUnit) -> &'static str {
    match unit {
        LengthUnit::Unspecified => "Unspecified",
        LengthUnit::Millimetre => "Millimetre",
        LengthUnit::Centimetre => "Centimetre",
        LengthUnit::Metre => "Metre",
        LengthUnit::Inch => "Inch",
        LengthUnit::Foot => "Foot",
    }
}

pub(crate) fn unit_from_name(name: &str) -> Option<LengthUnit> {
    Some(match name {
        "Unspecified" => LengthUnit::Unspecified,
        // Both spellings accepted on read: §4.2's own example writes
        // "Millimeter", and a file from either is the same scene. Only one is
        // written, so the round trip stays exact.
        "Millimetre" | "Millimeter" => LengthUnit::Millimetre,
        "Centimetre" | "Centimeter" => LengthUnit::Centimetre,
        "Metre" | "Meter" => LengthUnit::Metre,
        "Inch" => LengthUnit::Inch,
        "Foot" => LengthUnit::Foot,
        _ => return None,
    })
}

pub(crate) fn preset_name(preset: LightingPreset) -> &'static str {
    match preset {
        LightingPreset::Performance => "Performance",
        LightingPreset::Basic => "Basic",
        LightingPreset::Product => "Product",
        LightingPreset::Interior => "Interior",
        LightingPreset::Jewelry => "Jewelry",
    }
}

/// Whatever the capitalisation: a file writes "Jewelry", a person or a
/// script is as likely to type "jewelry".
pub(crate) fn preset_from_name(name: &str) -> Option<LightingPreset> {
    Some(match name.to_ascii_lowercase().as_str() {
        "performance" => LightingPreset::Performance,
        "basic" => LightingPreset::Basic,
        "product" => LightingPreset::Product,
        "interior" => LightingPreset::Interior,
        "jewelry" | "jewellery" => LightingPreset::Jewelry,
        _ => return None,
    })
}


// --- reading -------------------------------------------------------------

pub fn load(path: &Path) -> Result<Loaded, FormatError> {
    let bytes = std::fs::read(path)
        .map_err(|e| FormatError::Io(format!("{} could not be read: {e}", path.display())))?;
    read(&bytes)
}

pub fn read(bytes: &[u8]) -> Result<Loaded, FormatError> {
    let archive = zip::ZipArchive::read(bytes)?;
    let scene_bytes = archive
        .get(SCENE_MEMBER)
        .ok_or_else(|| FormatError::Invalid(format!("the archive has no `{SCENE_MEMBER}`")))?;
    let root = json::parse(scene_bytes)
        .map_err(|e| FormatError::Invalid(format!("{SCENE_MEMBER}: {e}")))?;

    let mut notes = Vec::new();
    let file_version = root
        .get("format_version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();
    check_version(&file_version, &mut notes)?;

    let mut document = SceneDocument::new();
    document.meta = DocumentMeta {
        format_version: FORMAT_VERSION.to_string(),
        application_version: env!("CARGO_PKG_VERSION").to_string(),
        name: root
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled")
            .to_string(),
    };
    let (units, metres_per_unit) = read_units(&root)?;
    document.units = units;
    document.environment = read_environment(&root);
    document.lighting = read_lighting(&root);
    document.cameras = read_cameras(&root);
    document.image_style = read_image_style(&root);
    document.ocio = read_color_management(&root);
    document.render = read_render_settings(&root);
    document.multi_materials = root
        .get("multi_materials")
        .map(|v| v.as_array().iter().filter_map(read_multi_material).collect())
        .unwrap_or_default();
    document.model_sets = root.get("model_sets").map(read_model_sets).unwrap_or_default();
    document.material_templates = root
        .get("material_templates")
        .map(|v| v.as_array().iter().map(read_template).collect())
        .unwrap_or_default();
    document.studios = root
        .get("studios")
        .map(|v| v.as_array().iter().map(read_studio).collect())
        .unwrap_or_default();
    if let Some(turn) = root.get("product_turn") {
        let pivot = turn.get("pivot").and_then(|p| p.as_f32_array()).unwrap_or_default();
        document.product_turn = tequila_scene::studio::ProductTurn {
            yaw: turn.get("yaw").and_then(|y| y.as_f32()).filter(|y| y.is_finite()).unwrap_or(0.0),
            pivot: [pivot.first().copied().unwrap_or(0.0), pivot.get(1).copied().unwrap_or(0.0)],
        };
    }
    document.animation = root.get("animation").map(read_animation);
    document.configurator = root.get("configurator").map(read_configurator).unwrap_or_default();
    document.measurements = root
        .get("measurements")
        .map(|v| v.as_array().iter().filter_map(read_measurement).collect())
        .unwrap_or_default();

    let mut preserved = Preserved::default();
    read_geometry(&archive, &mut document, &mut preserved)?;
    read_sources(&archive, &mut document);
    read_materials(&archive, &mut document, &mut preserved, &mut notes)?;
    read_tree(&root, &mut document, &mut preserved)?;
    preserved.settings = unknown_settings(&root, &document);

    // Watermarks, after the content, so `set_next_*` can only raise them.
    if let Some(next) = root.get("next_node_id").and_then(|v| v.as_index()) {
        document.tree.set_next_id(next as u64);
    }
    if let Some(next) = root.get("next_mesh_key").and_then(|v| v.as_index()) {
        document.geometry.set_next_key(next as u64);
    }
    if let Some(entries) = root.get("textures").map(|v| v.as_array()) {
        for entry in entries {
            let (Some(id), Some(path)) = (
                entry.get("id").and_then(|v| v.as_f32()),
                entry.get("path").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let role = entry
                .get("role")
                .and_then(|v| v.as_str())
                .and_then(journal::role_from_name)
                .unwrap_or(tequila_texture::texture::MapRole::Color);
            document.textures.insert_with_id(
                tequila_materials::graph::TextureId(id as u32),
                tequila_scene::textures::TextureSource {
                    path: std::path::PathBuf::from(path),
                    role,
                    color_space: entry
                        .get("color_space")
                        .and_then(|v| v.as_str())
                        .and_then(journal::color_space_from_name),
                },
            );
        }
    }
    if let Some(next) = root.get("next_texture_id").and_then(|v| v.as_index()) {
        document.textures.set_next_id(next as u32);
    }
    if let Some(next) = root.get("next_material_key").and_then(|v| v.as_index()) {
        document.materials.set_next_key(next as u64);
    }

    // Unknown top-level scene fields.
    const KNOWN_SCENE_FIELDS: [&str; 24] = [
        "format_version",
        "name",
        "units",
        // Still known, though no longer written: a document from before the
        // Lighting tab carries it, and preserving it as an unknown field would
        // put a stale preset back alongside the one that replaced it.
        "lighting_preset",
        "lighting",
        "environment",
        "cameras",
        "active_camera",
        "image_style",
        "color_management",
        "render",
        "measurements",
        "multi_materials",
        "model_sets",
        "studios",
        "product_turn",
        "configurator",
        "material_templates",
        "tree",
        "roots",
        "textures",
        "next_node_id",
        "next_mesh_key",
        "next_texture_id",
    ];
    if let Value::Object(map) = &root {
        for (key, value) in map {
            if key != "next_material_key" && !KNOWN_SCENE_FIELDS.contains(&key.as_str()) {
                preserved.scene.insert(key.clone(), value.clone());
            }
        }
    }
    for name in archive.names() {
        if name != SCENE_MEMBER
            && name != MANIFEST_MEMBER
            && !name.starts_with("geometry/")
            && !name.starts_with("materials/")
            && !name.starts_with("sources/")
        {
            preserved
                .members
                .insert(name.clone(), archive.get(name).unwrap_or_default().to_vec());
        }
    }
    if let Some(summary) = preserved.summary() {
        notes.push(summary);
    }

    // Until 2026-10-13 a file could keep its lengths in the document's unit,
    // with `metres_per_unit` saying what one was worth. The readouts
    // honoured it and the renderer never did: a millimetre scene rendered a
    // thousand times its size, with every material's pattern a thousandth
    // of its own. Brought into metres the way Scale Scene does it, so the
    // model comes out the size the file said and the shot keeps its framing
    // and brightness.
    if (metres_per_unit - 1.0).abs() > 1e-9 {
        tequila_scene::units::rescale(&mut document, metres_per_unit);
        notes.push(format!(
            "lengths were stored in units of {metres_per_unit} m; converted to metres"
        ));
    }

    document
        .validate()
        .map_err(|e| FormatError::Invalid(format!("the loaded scene is inconsistent: {e}")))?;
    Ok(Loaded {
        document,
        preserved,
        notes,
    })
}

/// §4.4's loading rules, in order.
fn check_version(file: &str, notes: &mut Vec<String>) -> Result<(), FormatError> {
    let parse = |text: &str| -> (u64, u64) {
        let mut parts = text.split('.');
        (
            parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
            parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
        )
    };
    let (file_major, file_minor) = parse(file);
    let (major, minor) = parse(FORMAT_VERSION);

    if file_major > major {
        return Err(FormatError::TooNew {
            file: file.to_string(),
            supported: FORMAT_VERSION.to_string(),
        });
    }
    if file_major < major {
        // The hook §4.4 asks for: "Write a migration function per version
        // bump". There has been no major bump yet, so there is nothing to run;
        // when there is, it dispatches here and appends what it did.
        notes.push(format!(
            "migrated from format {file} to {FORMAT_VERSION}: no changes were needed"
        ));
    } else if file_minor > minor {
        notes.push(format!(
            "this file uses format {file}; this build writes {FORMAT_VERSION}. \
             Fields it does not understand are preserved."
        ));
    }
    Ok(())
}

/// The unit, and how many metres the file's lengths are each: 1 for every
/// file written since lengths were always metres, and anything for one from
/// before, which [`read`] then scales into metres.
fn read_units(root: &Value) -> Result<(SceneUnits, f64), FormatError> {
    let Some(units) = root.get("units") else {
        return Ok((SceneUnits::default(), 1.0));
    };
    let name = units
        .get("unit")
        .and_then(|v| v.as_str())
        .unwrap_or("Unspecified");
    let unit = unit_from_name(name)
        .ok_or_else(|| FormatError::Invalid(format!("`{name}` is not a length unit")))?;
    let metres_per_unit = units
        .get("metres_per_unit")
        .or_else(|| units.get("meters_per_unit"))
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);
    if !(metres_per_unit.is_finite() && metres_per_unit > 0.0) {
        return Err(FormatError::Invalid(format!(
            "metres_per_unit is {metres_per_unit}, which is not a usable scale"
        )));
    }
    Ok((SceneUnits::new(unit), metres_per_unit))
}

fn read_environment(root: &Value) -> EnvironmentSettings {
    let mut settings = EnvironmentSettings::default();
    let Some(environment) = root.get("environment") else {
        return settings;
    };
    settings.image = environment
        .get("image")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    settings.sky = environment.get("sky").map(sky_from_json);
    if let Some(pins) = environment.get("pins").map(|v| v.as_array()) {
        settings.pins = pins.iter().map(pin_from_json).collect();
    }

    for param in EnvParam::ALL {
        let (section, field) = param.json_path();
        let holder = if section.is_empty() {
            Some(environment)
        } else {
            environment.get(section)
        };
        let Some(value) = holder.and_then(|h| h.get(field)) else {
            continue;
        };
        // Shaped by what the field *is*, not by what the JSON happens to hold:
        // a number where a flag belongs is a broken file, and taking it as a
        // flag would be inventing a value rather than falling back to the
        // default.
        let read = match param.read(&settings) {
            EnvValue::Scalar(_) => value.as_f32().map(EnvValue::Scalar),
            EnvValue::Flag(_) => value.as_bool().map(EnvValue::Flag),
            EnvValue::Color(_) => read_spectrum(value).map(EnvValue::Color),
            EnvValue::Mode(_) => value
                .as_str()
                .and_then(|s| BackgroundMode::ALL.into_iter().find(|m| m.label() == s))
                .map(EnvValue::Mode),
            EnvValue::Path(_) => Some(EnvValue::Path(value.as_str().map(|s| s.to_string()))),
            EnvValue::Choice(_) => value
                .as_str()
                .and_then(|s| param.choices().iter().find(|(key, _)| *key == s))
                .map(|(key, _)| EnvValue::Choice(key)),
        };
        if let Some(read) = read {
            param.write(&mut settings, &read);
        }
    }
    settings
}

fn read_sources(archive: &zip::ZipArchive, document: &mut SceneDocument) {
    for name in archive.names() {
        let Some(id) = name.strip_prefix("sources/").and_then(|n| n.strip_suffix(".step").or_else(|| n.strip_suffix(".iges"))) else { continue };
        document.sources.insert(id.to_string(), std::sync::Arc::new(archive.get(name).unwrap_or_default().to_vec()));
    }
}

fn read_geometry(
    archive: &zip::ZipArchive,
    document: &mut SceneDocument,
    _preserved: &mut Preserved,
) -> Result<(), FormatError> {
    for name in archive.names() {
        let Some(stem) = name
            .strip_prefix("geometry/")
            .and_then(|n| n.strip_suffix(".fsmesh"))
        else {
            continue;
        };
        let key = parse_id("m_", stem)
            .ok_or_else(|| FormatError::Invalid(format!("`{name}` is not a mesh identifier")))?;
        let bytes = archive.get(name).unwrap_or_default();
        let mesh = fsmesh::read(bytes).map_err(|e| FormatError::Mesh(format!("{name}: {e}")))?;
        document.geometry.insert_with_key(MeshKey(key), mesh);
    }
    Ok(())
}

fn read_materials(
    archive: &zip::ZipArchive,
    document: &mut SceneDocument,
    preserved: &mut Preserved,
    notes: &mut Vec<String>,
) -> Result<(), FormatError> {
    let mut newer = Vec::new();
    for name in archive.names() {
        let Some(stem) = name
            .strip_prefix("materials/")
            .and_then(|n| n.strip_suffix(".json"))
        else {
            continue;
        };
        let key = parse_id("mat_", stem).ok_or_else(|| {
            FormatError::Invalid(format!("`{name}` is not a material identifier"))
        })?;
        let value = json::parse(archive.get(name).unwrap_or_default())
            .map_err(|e| FormatError::Invalid(format!("{name}: {e}")))?;
        let (definition, extras) = read_material_keeping(&value, name)?;
        if extras.loaded.is_some() {
            newer.push(definition.name.clone());
        }
        if extras != MaterialExtras::default() {
            preserved.materials.insert(key, extras);
        }
        document.materials.insert_with_key(MaterialKey(key), definition);
    }
    if !newer.is_empty() {
        notes.push(format!(
            "{} use a material type or nodes from a newer Tequila: shown with their settings alone, and saved as they were unless you change them",
            newer.join(", ")
        ));
    }
    Ok(())
}

/// A project's material, read as far as this build can: a type or a graph
/// it does not know does not stop the project opening - the material is
/// shown with its settings alone, and what could not be read is kept with
/// whatever else it carried this build does not know.
fn read_material_keeping(value: &Value, name: &str) -> Result<(tequila_scene::material::MaterialDefinition, MaterialExtras), FormatError> {
    let mut extras = MaterialExtras::default();
    if let Value::Object(map) = value {
        extras.unknown = map
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "id" | "name" | "type" | "graph") && !Param::ALL.iter().any(|p| p.key() == key.as_str()))
            .map(|(key, item)| (key.clone(), item.clone()))
            .collect();
    }
    let mut value = value.clone();
    if let Value::Object(map) = &mut value {
        let kind = map.get("type").and_then(|v| v.as_str()).map(str::to_string);
        if let Some(kind) = kind.filter(|k| MaterialType::from_id(k).is_none()) {
            map.insert("type".into(), Value::String(MaterialType::Generic.id().into()));
            extras.original_type = Some(kind);
        }
    }
    let definition = match read_material(&value, name) {
        Ok(definition) => definition,
        Err(_) if value.get("graph").is_some() => {
            let graph = value.get("graph").cloned();
            if let Value::Object(map) = &mut value {
                map.remove("graph");
            }
            let definition = read_material(&value, name)?;
            extras.original_graph = graph;
            definition
        }
        Err(error) => return Err(error),
    };
    if extras.original_type.is_some() || extras.original_graph.is_some() {
        extras.loaded = Some(definition.clone());
    }
    Ok((definition, extras))
}

/// A material from its JSON, as `material_to_json` writes it: what a USD
/// file carries as `tequila:definition`.
pub fn material_from_json(text: &str) -> Result<MaterialDefinition, FormatError> {
    let value = json::parse(text.as_bytes()).map_err(|e| FormatError::Invalid(e.to_string()))?;
    read_material(&value, "a material definition")
}

pub(crate) fn read_material(value: &Value, name: &str) -> Result<MaterialDefinition, FormatError> {
    let material_name = value
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Material")
        .to_string();
    let type_text = value
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("plastic");
    let material_type = MaterialType::from_id(type_text).ok_or_else(|| {
        FormatError::Invalid(format!("{name}: `{type_text}` is not a material type"))
    })?;

    // Start from the type's defaults, so a file written before a parameter
    // existed gets a value that suits the type rather than a global default
    // that may make no sense for it.
    let mut material = material_type.defaults();
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
            ParamValue::Color(_) => read_spectrum(present).map(ParamValue::Color),
            ParamValue::Flag(_) => present.as_bool().map(ParamValue::Flag),
            ParamValue::Metal(fallback) => Some(ParamValue::Metal(ComplexIor {
                n: present.get("n").and_then(read_spectrum).unwrap_or(fallback.n),
                k: present.get("k").and_then(read_spectrum).unwrap_or(fallback.k),
            })),
        };
        // A malformed value falls back to the preset's rather than failing the
        // open: a scene with one bad number still opens.
        if let Some(parsed) = parsed {
            param.write(&mut material, parsed);
        }
    }

    let graph = graph::read_graph(value.get("graph"), material.sanitized(), name)?;
    Ok(MaterialDefinition::from_graph(
        material_name,
        material_type,
        graph,
    ))
}

pub(crate) fn read_spectrum(value: &Value) -> Option<Spectrum> {
    if let Some(text) = value.as_str() {
        return hex_colour(text);
    }
    value
        .as_f32_array()
        .filter(|v| v.len() >= 3)
        .map(|v| spectrum(v[0], v[1], v[2]))
}

/// `"#c0392b"` or `"#c33"`: a colour as a person or a model picks one,
/// sRGB-encoded, brought into the rendering space as every picker does
/// (`docs` on colour: ACEScg). An array is already in that space; a hex
/// colour is what a recipe written by hand or by an AI app most often has.
pub fn hex_colour(text: &str) -> Option<Spectrum> {
    let hex = text.trim().strip_prefix('#')?;
    let digits: Vec<u8> = hex.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
    let [r, g, b] = match digits.as_slice() {
        [r1, r0, g1, g0, b1, b0] => [r1 * 16 + r0, g1 * 16 + g0, b1 * 16 + b0],
        [r, g, b] => [r * 17, g * 17, b * 17],
        _ => return None,
    };
    let linear = |c: u8| {
        let v = c as f32 / 255.0;
        if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    Some(Spectrum::from_linear_srgb(tequila_core::math::vec3(linear(r), linear(g), linear(b))))
}

fn read_tree(
    root: &Value,
    document: &mut SceneDocument,
    preserved: &mut Preserved,
) -> Result<(), FormatError> {
    // Every field `node_to_json` writes, or a project opened in the build
    // that wrote it reports itself as from a newer one.
    const KNOWN_NODE_FIELDS: [&str; 13] = [
        "id",
        "name",
        "kind",
        "parent",
        "transform",
        "visible",
        "locked",
        "children",
        "mesh",
        "light",
        "cutaway",
        "material",
        "metadata",
    ];

    let nodes = root
        .get("tree")
        .map(|t| t.as_array())
        .ok_or_else(|| FormatError::Invalid("the scene has no `tree`".into()))?;

    let mut built = Vec::with_capacity(nodes.len());
    for value in nodes {
        let id = value
            .get("id")
            .and_then(|v| v.as_index())
            .ok_or_else(|| FormatError::Invalid("a node has no `id`".into()))?
            as u64;
        let name = value.get("name").and_then(|v| v.as_str()).unwrap_or("Node");

        let kind = match value.get("kind").and_then(|v| v.as_str()) {
            Some("Part") => {
                let mesh = value
                    .get("mesh")
                    .and_then(|v| v.as_str())
                    .and_then(|s| parse_id("m_", s));
                let mesh = mesh.ok_or_else(|| {
                    FormatError::Invalid(format!("node {id} is a Part but names no mesh"))
                })?;
                NodeKind::Part {
                    mesh: MeshKey(mesh),
                }
            }
            Some("Group") | None => NodeKind::Group,
            Some("Cutaway") => NodeKind::Cutaway {
                settings: Box::new(read_cutaway(value.get("cutaway").ok_or_else(|| {
                    FormatError::Invalid(format!("node {id} is a Cutaway but has no settings"))
                })?)),
            },
            // The kind of a light is the *shape* it is — "Area", "Spot" — so
            // the Scene tab can label the row without opening the light. That
            // makes the shape names part of the format, and reading them back
            // through the same table is what keeps the two ends agreeing.
            Some(other) if value.get("light").is_some() => {
                let settings = read_light(value.get("light").expect("checked above"));
                if settings.shape.label() != other {
                    return Err(FormatError::Invalid(format!(
                        "node {id} is labelled `{other}` but its light is a {}",
                        settings.shape.label()
                    )));
                }
                NodeKind::Light {
                    settings: Box::new(settings),
                }
            }
            Some(other) => {
                return Err(FormatError::Invalid(format!(
                    "node {id} has unknown kind `{other}`"
                )))
            }
        };

        let mut node = SceneNode::new(NodeId(id), name, kind);
        node.parent = value
            .get("parent")
            .and_then(|v| v.as_index())
            .map(|p| NodeId(p as u64));
        node.transform = read_transform(value, id)?;
        node.visible = value
            .get("visible")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        node.locked = value
            .get("locked")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        node.rays = read_rays(value.get("rays"));
        node.material = value
            .get("material")
            .and_then(|v| v.as_str())
            .and_then(|s| parse_id("mat_", s))
            .map(MaterialKey);
        node.children = value
            .get("children")
            .map(|c| c.as_array())
            .unwrap_or(&[])
            .iter()
            .filter_map(|v| v.as_index())
            .map(|c| NodeId(c as u64))
            .collect();
        node.path_key = value
            .get("path_key")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        node.pivot = value
            .get("pivot")
            .and_then(|v| v.as_f32_array())
            .filter(|v| v.len() == 3)
            .map(|v| tequila_core::math::vec3(v[0], v[1], v[2]));
        if let Some(Value::Object(map)) = value.get("metadata") {
            for (key, item) in map {
                if let Some(text) = item.as_str() {
                    node.metadata.insert(key.clone(), text.to_string());
                }
            }
        }

        if let Value::Object(map) = value {
            let unknown: BTreeMap<String, Value> = map
                .iter()
                .filter(|(k, _)| {
                    !matches!(k.as_str(), "path_key" | "pivot")
                        && !KNOWN_NODE_FIELDS.contains(&k.as_str())
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if !unknown.is_empty() {
                preserved.nodes.insert(id, unknown);
            }
        }
        built.push(node);
    }

    let roots: Vec<NodeId> = root
        .get("roots")
        .map(|r| r.as_array())
        .unwrap_or(&[])
        .iter()
        .filter_map(|v| v.as_index())
        .map(|r| NodeId(r as u64))
        .collect();

    document
        .tree
        .restore(built, roots)
        .map_err(|e| FormatError::Invalid(format!("the tree does not hold together: {e}")))?;
    Ok(())
}

fn read_transform(value: &Value, id: u64) -> Result<Transform, FormatError> {
    let Some(values) = value.get("transform").and_then(|v| v.as_f32_array()) else {
        return Ok(Transform::IDENTITY);
    };
    if values.len() != 16 {
        return Err(FormatError::Invalid(format!(
            "node {id} has a transform with {} values, not 16",
            values.len()
        )));
    }
    let mut matrix = Mat4::IDENTITY;
    for row in 0..4 {
        for column in 0..4 {
            matrix.m[row][column] = values[row * 4 + column];
        }
    }
    Transform::from_matrix(matrix).ok_or_else(|| {
        FormatError::Invalid(format!(
            "node {id} has a transform that cannot be inverted; it would collapse the part"
        ))
    })
}

#[cfg(test)]
mod forward_compatibility_tests {
    use super::*;

    /// The archive at `path` with its members passed through `edit`.
    fn rewrite(path: &Path, edit: &dyn Fn(&str, Value) -> Value) {
        let bytes = std::fs::read(path).unwrap();
        let archive = zip::ZipArchive::read(&bytes).unwrap();
        let mut writer = zip::ZipWriter::new();
        for name in archive.names() {
            let member = archive.get(name).unwrap();
            if name.ends_with(".json") {
                let edited = edit(name, json::parse(member).unwrap());
                writer.add(name.clone(), json_write::to_string(&edited).into_bytes());
            } else {
                writer.add(name.clone(), member.to_vec());
            }
        }
        std::fs::write(path, writer.finish()).unwrap();
    }

    fn member(path: &Path, name: &str) -> Value {
        let bytes = std::fs::read(path).unwrap();
        let archive = zip::ZipArchive::read(&bytes).unwrap();
        json::parse(archive.get(name).unwrap()).unwrap()
    }

    fn set(value: &mut Value, path: &[&str], key: &str, item: Value) {
        let mut current = value;
        for part in path {
            current = match current {
                Value::Object(map) => map.get_mut(*part).unwrap(),
                _ => panic!("no {part}"),
            };
        }
        if let Value::Object(map) = current {
            map.insert(key.into(), item);
        }
    }

    /// A project as a newer Tequila might write it - new settings, a new
    /// material parameter, a material type and a graph node this build has
    /// never heard of - opens, and saves with all of it as it was.
    #[test]
    fn a_newer_projects_settings_and_materials_survive_a_save() {
        let directory = std::env::temp_dir().join(format!("tequila-forward-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("newer.tequila");
        let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
        let key = document.materials.iter().next().unwrap().0;
        save(&path, &document, &Preserved::default()).unwrap();
        let material_member = format!("materials/{}.json", material_id(key));
        let future_graph = Value::Object(BTreeMap::from([
            ("nodes".to_string(), Value::Array(vec![Value::Object(BTreeMap::from([
                ("id".to_string(), Value::Number(9.0)),
                ("kind".to_string(), Value::String("quantum_foam".into())),
            ]))])),
            ("links".to_string(), Value::Array(Vec::new())),
            ("output".to_string(), Value::Number(1.0)),
        ]));
        rewrite(&path, &|name, mut value| {
            if name == SCENE_MEMBER {
                set(&mut value, &["environment", "ground"], "caustic_glow", Value::Number(0.5));
                set(&mut value, &["environment"], "aurora", Value::String("north".into()));
                set(&mut value, &["render"], "photon_budget", Value::Number(7.0));
                set(&mut value, &["lighting", "custom"], "path_guiding", Value::Bool(true));
                set(&mut value, &["image_style"], "film_grain", Value::Number(0.2));
                set(&mut value, &["units"], "meters_per_unit", Value::Number(1.0));
            } else if name == material_member {
                set(&mut value, &[], "iridescence_spin", Value::Number(0.4));
                set(&mut value, &[], "type", Value::String("hologram".into()));
                set(&mut value, &[], "graph", future_graph.clone());
            }
            value
        });

        let loaded = load(&path).expect("a newer project opens");
        assert!(loaded.notes.iter().any(|n| n.contains("newer Tequila")), "{:?}", loaded.notes);
        let again = directory.join("saved by this build.tequila");
        save(&again, &loaded.document, &loaded.preserved).unwrap();
        let scene = member(&again, SCENE_MEMBER);
        assert_eq!(scene.get("environment").and_then(|e| e.get("ground")).and_then(|g| g.get("caustic_glow")), Some(&Value::Number(0.5)));
        assert_eq!(scene.get("environment").and_then(|e| e.get("aurora")).and_then(Value::as_str), Some("north"));
        assert_eq!(scene.get("render").and_then(|r| r.get("photon_budget")), Some(&Value::Number(7.0)));
        assert_eq!(scene.get("lighting").and_then(|l| l.get("custom")).and_then(|c| c.get("path_guiding")), Some(&Value::Bool(true)));
        assert_eq!(scene.get("image_style").and_then(|i| i.get("film_grain")), Some(&Value::Number(0.2)));
        assert!(scene.get("units").and_then(|u| u.get("meters_per_unit")).is_none(), "an old key this build reads came back as a newer one");
        let material = member(&again, &material_member);
        assert_eq!(material.get("iridescence_spin"), Some(&Value::Number(0.4)));
        assert_eq!(material.get("type").and_then(Value::as_str), Some("hologram"));
        assert_eq!(material.get("graph"), Some(&future_graph));

        // Changed here, the material is this build's from then on; what it
        // did not understand of the rest is still kept.
        let mut changed = loaded.document.clone();
        let mut definition = changed.materials.get(key).unwrap().clone();
        definition.name = "Renamed in an older build".into();
        changed.materials.insert_with_key(key, definition);
        save(&again, &changed, &loaded.preserved).unwrap();
        let material = member(&again, &material_member);
        assert_eq!(material.get("type").and_then(Value::as_str), Some("generic"));
        assert_ne!(material.get("graph"), Some(&future_graph));
        assert_eq!(material.get("iridescence_spin"), Some(&Value::Number(0.4)));
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// What this build knows and the person cleared is not brought back
    /// from the file it was loaded from: a time limit, a backplate.
    #[test]
    fn a_cleared_known_setting_stays_cleared() {
        let directory = std::env::temp_dir().join(format!("tequila-forward-cleared-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("set.tequila");
        let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
        document.render.max_seconds = Some(600);
        document.environment.image = Some("C:/studio.hdr".into());
        save(&path, &document, &Preserved::default()).unwrap();
        let mut loaded = load(&path).unwrap();
        assert!(loaded.preserved.is_empty(), "this build's own file had something it did not know: {:?}", loaded.preserved);
        loaded.document.render.max_seconds = None;
        loaded.document.environment.image = None;
        save(&path, &loaded.document, &loaded.preserved).unwrap();
        let reloaded = load(&path).unwrap().document;
        assert_eq!(reloaded.render.max_seconds, None);
        assert_eq!(reloaded.environment.image, None);
        let _ = std::fs::remove_dir_all(&directory);
    }
}

#[cfg(test)]
mod display_default_tests {
    use super::*;

    /// New scenes get PBR Neutral; a file saved before the transform was
    /// written keeps AgX, which it was rendered with — changing the default
    /// must not change how an old project looks.
    #[test]
    fn new_scenes_are_neutral_and_old_files_keep_agx() {
        assert_eq!(ImageStyle::default().display, DisplayTransform::PbrNeutral);
        let old = json::parse(br#"{"type":"Basic","exposure":0}"#).unwrap();
        assert_eq!(read_image_style_value(&old).display, DisplayTransform::AgX);
        let saved = image_style_to_json(&ImageStyle {
            display: DisplayTransform::Aces,
            ..ImageStyle::default()
        });
        assert_eq!(read_image_style_value(&saved).display, DisplayTransform::Aces);
    }
}

#[cfg(test)]
mod engine_tests {
    use super::*;
    use tequila_scene::render::RenderEngine;

    /// The engine a project renders with is saved with it, and a project
    /// from before there was a choice renders with the default engine,
    /// Cycles, as a new project does.
    #[test]
    fn the_engine_is_kept_and_old_projects_take_the_default() {
        for engine in RenderEngine::ALL {
            let saved = render_settings_to_json(&RenderSettings { engine, ..RenderSettings::default() });
            assert_eq!(read_render_settings_value(&saved).engine, engine);
        }
        let old = json::parse(br#"{"resolution":[640,480],"max_samples":64}"#).unwrap();
        assert_eq!(read_render_settings_value(&old).engine, RenderEngine::CyclesGpu);
        let unknown = json::parse(br#"{"engine":"lux"}"#).unwrap();
        assert_eq!(read_render_settings_value(&unknown).engine, RenderEngine::CyclesGpu);
    }
}

#[cfg(test)]
mod hex_colour_tests {
    use super::*;

    /// A hex colour is sRGB: white stays white, mid-grey decodes to its
    /// linear value, a pure red arrives as the rendering space holds it, and
    /// anything else is not a colour.
    #[test]
    fn hex_colours_are_decoded_from_srgb() {
        let close = |a: Spectrum, b: Spectrum| (a.r - b.r).abs() < 1e-3 && (a.g - b.g).abs() < 1e-3 && (a.b - b.b).abs() < 1e-3;
        assert!(close(hex_colour("#ffffff").unwrap(), Spectrum::from_linear_srgb(tequila_core::math::vec3(1.0, 1.0, 1.0))));
        let grey = hex_colour("#808080").unwrap();
        assert!((grey.r - 0.2158).abs() < 1e-3 && (grey.g - grey.b).abs() < 1e-4, "{grey:?}");
        assert!(close(hex_colour("#f00").unwrap(), Spectrum::from_linear_srgb(tequila_core::math::vec3(1.0, 0.0, 0.0))));
        assert_eq!(read_spectrum(&json::Value::String("#00ff00".into())), hex_colour("#00FF00"));
        for wrong in ["ff0000", "#ff00", "#gg0000", "red"] {
            assert!(hex_colour(wrong).is_none(), "{wrong}");
        }
    }

    /// A Multi-Material's variants are named as a file names materials or
    /// as every command and query numbers them; a swatch may be hex, kept
    /// as the panel's picker keeps it, in linear sRGB. Dropped silently, an
    /// AI app's numbered variants left a colourway with nothing in it.
    #[test]
    fn variants_by_number_and_swatches_in_hex() {
        let multi = read_multi_material(&json::parse(br#"{"id":1,"name":"Body","parts":[3],"variants":["mat_00000002",5],"active":1}"#).unwrap()).unwrap();
        assert_eq!(multi.variants, vec![MaterialKey(2), MaterialKey(5)]);
        let configurator = read_configurator(
            &json::parse(br##"{"options":[{"id":1,"name":"Body","drives":"material","multi_material":1,"choices":[{"label":"Red","target":0,"swatch":"#ff0000"},{"label":"Grey","target":1,"swatch":[0.2,0.2,0.2]}]}]}"##).unwrap(),
        );
        let choices = &configurator.options[0].choices;
        let red = choices[0].swatch.expect("a hex swatch");
        assert!((red[0] - 1.0).abs() < 1e-3 && red[1].abs() < 1e-3 && red[2].abs() < 1e-3, "{red:?}");
        assert_eq!(choices[1].swatch, Some([0.2, 0.2, 0.2]));
    }
}

#[cfg(test)]
mod light_link_tests {
    use super::*;
    use tequila_scene::lighting::LightLinking;

    /// A light's linking is kept in the file, and a light without one
    /// writes nothing new: a file from before reads as it was.
    #[test]
    fn a_lights_links_survive_a_round_trip() {
        let linked = LightSettings {
            links: Some(LightLinking { exclude: true, nodes: vec![tequila_scene::tree::NodeId(4), tequila_scene::tree::NodeId(9)], product: false }),
            ..LightSettings::default()
        };
        assert_eq!(read_light(&light_to_json(&linked)), linked);
        let product = LightSettings { links: Some(LightLinking { product: true, ..LightLinking::default() }), ..LightSettings::default() };
        assert_eq!(read_light(&light_to_json(&product)), product);
        let plain = json_write::to_string(&light_to_json(&LightSettings::default()));
        assert!(!plain.contains("links"), "{plain}");
    }

    /// A camera's physical exposure is kept, and one without writes none.
    #[test]
    fn a_cameras_physical_exposure_survives_a_round_trip() {
        use tequila_scene::lighting::SavedCamera;
        let saved = SavedCamera {
            name: "Shot".into(),
            camera: tequila_render_core::camera::Camera { physical_exposure: true, iso: 400.0, ..Default::default() },
            target: tequila_core::math::Vec3::ZERO,
        };
        let text = json_write::to_string(&camera_to_json(&saved));
        assert!(text.contains("physical_exposure"), "{text}");
        let back = read_camera(&json::parse(text.as_bytes()).unwrap());
        assert!(back.camera.physical_exposure && back.camera.iso == 400.0);
        let plain = SavedCamera { camera: Default::default(), ..saved };
        assert!(!json_write::to_string(&camera_to_json(&plain)).contains("physical_exposure"));
    }

    /// A sun keeps its angle and its lux.
    #[test]
    fn a_sun_survives_a_round_trip() {
        let sun = LightSettings {
            shape: tequila_scene::lighting::LightShape::Sun { angle_degrees: 2.5 },
            intensity: tequila_scene::lighting::Intensity::Lux(800.0),
            ..LightSettings::default()
        };
        assert_eq!(read_light(&light_to_json(&sun)), sun);
    }

    /// A softbox's spread is kept, and a plain panel writes none.
    #[test]
    fn an_area_lights_spread_survives_a_round_trip() {
        let gridded = LightSettings { spread_degrees: 40.0, ..LightSettings::default() };
        assert_eq!(read_light(&light_to_json(&gridded)), gridded);
        let plain = json_write::to_string(&light_to_json(&LightSettings::default()));
        assert!(!plain.contains("spread"), "{plain}");
    }

    /// Shadow links are kept beside the light's own, and apart from them.
    #[test]
    fn a_lights_shadow_links_survive_a_round_trip() {
        let shadows = LightSettings {
            links: Some(LightLinking { product: true, ..LightLinking::default() }),
            shadow_links: Some(LightLinking { exclude: true, nodes: vec![tequila_scene::tree::NodeId(7)], product: false }),
            ..LightSettings::default()
        };
        let back = read_light(&light_to_json(&shadows));
        assert_eq!(back, shadows);
        let only = LightSettings { shadow_links: shadows.shadow_links.clone(), ..LightSettings::default() };
        assert_eq!(read_light(&light_to_json(&only)).links, None, "shadow links became light links");
    }
}

#[cfg(test)]
mod rays_tests {
    use super::*;
    use tequila_scene::tree::Rays;

    /// A node's rays are kept in the file, and a node seen by every ray
    /// writes nothing new.
    #[test]
    fn a_nodes_rays_survive_and_default_ones_write_nothing() {
        let rays = Rays { camera: false, transmission: false, holdout: true, ..Rays::default() };
        assert_eq!(read_rays(Some(&rays_to_json(&rays))), rays);
        assert_eq!(read_rays(None), Rays::default());
        let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
        let plain = json_write::to_string(&node_to_json(document.tree.iter().next().unwrap(), &Preserved::default()));
        assert!(!plain.contains("\"rays\""), "{plain}");
        let mut node = document.tree.iter().next().unwrap().clone();
        node.rays = rays;
        let written = node_to_json(&node, &Preserved::default());
        assert_eq!(read_rays(written.get("rays")), rays);
    }
}
